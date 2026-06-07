# -------------------------------------------------------------------------------------------------
#  Copyright (C) 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
#  https://nautechsystems.io
#
#  Licensed under the GNU Lesser General Public License Version 3.0 (the "License");
#  You may not use this file except in compliance with the License.
#  You may obtain a copy of the License at https://www.gnu.org/licenses/lgpl-3.0.en.html
#
#  Unless required by applicable law or agreed to in writing, software
#  distributed under the License is distributed on an "AS IS" BASIS,
#  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
#  See the License for the specific language governing permissions and
#  limitations under the License.
# -------------------------------------------------------------------------------------------------
"""
Declarative strategy processor for externally owned JSON strategies.

``JsonStrategy`` is not an application strategy. It is an adapter that interprets strategy JSON
submitted with a backtest API request, so this repository can process strategies without owning
strategy source files.
"""

from __future__ import annotations

from typing import Any

from nautilus_trader.model.data import Bar
from nautilus_trader.model.data import BarType
from nautilus_trader.model.data import QuoteTick
from nautilus_trader.model.data import TradeTick
from nautilus_trader.model.enums import OrderSide
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.trading.config import StrategyConfig
from nautilus_trader.trading.strategy import Strategy


class JsonStrategyConfig(StrategyConfig, frozen=True):
    """
    Configuration for :class:`JsonStrategy`.

    Parameters
    ----------
    spec : dict[str, Any]
        The caller-owned strategy JSON document.

    """

    spec: dict[str, Any]


class JsonStrategy(Strategy):
    """
    Processes caller-owned declarative strategy JSON.

    Supported action types
    ----------------------
    ``subscribe_bars``
        Requires ``bar_type``.
    ``subscribe_quote_ticks``
        Requires ``instrument_id``.
    ``subscribe_trade_ticks``
        Requires ``instrument_id``.
    ``market_order``
        Requires ``instrument_id``, ``side`` and ``quantity``. Quantity is converted via the cached
        instrument, so callers can supply the same primitive number/string they would use in a JSON
        API.
    ``log``
        Requires ``message`` and optionally ``level``.

    """

    def __init__(self, config: JsonStrategyConfig) -> None:
        super().__init__(config)
        self._spec = config.spec
        self._event_counts: dict[str, int] = {}

    def on_start(self) -> None:
        self._run_actions("on_start")

    def on_stop(self) -> None:
        self._run_actions("on_stop")

    def on_bar(self, bar: Bar) -> None:
        self._run_actions("on_bar", event=bar)

    def on_quote_tick(self, tick: QuoteTick) -> None:
        self._run_actions("on_quote_tick", event=tick)

    def on_trade_tick(self, tick: TradeTick) -> None:
        self._run_actions("on_trade_tick", event=tick)

    def on_reset(self) -> None:
        self._event_counts.clear()

    def _run_actions(self, event_name: str, event: Any | None = None) -> None:
        self._event_counts[event_name] = self._event_counts.get(event_name, 0) + 1
        for action in self._actions_for_event(event_name):
            if self._should_run(action=action, event_name=event_name):
                self._run_action(action=action, event=event)

    def _actions_for_event(self, event_name: str) -> list[dict[str, Any]]:
        actions = self._spec.get(event_name, [])
        if isinstance(actions, dict):
            return [actions]
        if not isinstance(actions, list):
            raise ValueError(
                f"Expected '{event_name}' to be an action object or list of action objects"
            )
        return actions

    def _should_run(self, action: dict[str, Any], event_name: str) -> bool:
        once = bool(action.get("once", False))
        if once and self._event_counts[event_name] > 1:
            return False

        only_on = action.get("only_on")
        return only_on is None or int(only_on) == self._event_counts[event_name]

    def _run_action(self, action: dict[str, Any], event: Any | None) -> None:
        action_type = action.get("type")
        if action_type == "subscribe_bars":
            self.subscribe_bars(BarType.from_str(action["bar_type"]))
        elif action_type == "subscribe_quote_ticks":
            self.subscribe_quote_ticks(InstrumentId.from_str(action["instrument_id"]))
        elif action_type == "subscribe_trade_ticks":
            self.subscribe_trade_ticks(InstrumentId.from_str(action["instrument_id"]))
        elif action_type == "market_order":
            self._submit_market_order(action)
        elif action_type == "log":
            self._log(action=action, event=event)
        else:
            raise ValueError(f"Unsupported JSON strategy action type: {action_type!r}")

    def _submit_market_order(self, action: dict[str, Any]) -> None:
        instrument_id = InstrumentId.from_str(action["instrument_id"])
        instrument = self.cache.instrument(instrument_id)
        if instrument is None:
            raise ValueError(f"No instrument loaded for JSON strategy action: {instrument_id}")

        self.submit_order(
            self.order_factory.market(
                instrument_id=instrument_id,
                order_side=OrderSide[action["side"].upper()],
                quantity=instrument.make_qty(action["quantity"]),
            ),
        )

    def _log(self, action: dict[str, Any], event: Any | None) -> None:
        message = str(action.get("message", ""))
        if action.get("include_event", False) and event is not None:
            message = f"{message} {event}"

        level = str(action.get("level", "info")).lower()
        if level == "debug":
            self.log.debug(message)
        elif level == "warning":
            self.log.warning(message)
        elif level == "error":
            self.log.error(message)
        else:
            self.log.info(message)
