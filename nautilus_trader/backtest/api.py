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
Callable JSON API for backtest requests.

The API in this module lets external applications submit a complete backtest request as JSON.
The caller owns the strategy definition; this package only adapts the submitted JSON into the
runtime strategy processor used by the backtest engine.

Request shape
-------------
A request may be either a single object or a list of objects. Each object supports:

``run_config``
    A JSON object with the normal :class:`BacktestRunConfig` fields, except that
    ``engine.strategies`` may be omitted.
``strategies``
    A list of declarative strategy JSON objects processed by ``JsonStrategy``.

The strategy JSON intentionally remains generic. The current processor supports event action
lists under keys such as ``on_start``, ``on_bar``, ``on_quote_tick`` and ``on_trade_tick``.
"""

from __future__ import annotations

from copy import deepcopy
from typing import Any

import msgspec

from nautilus_trader.backtest.config import BacktestRunConfig
from nautilus_trader.backtest.node import BacktestNode
from nautilus_trader.common.config import msgspec_decoding_hook
from nautilus_trader.common.config import msgspec_encoding_hook
from nautilus_trader.trading.config import ImportableStrategyConfig


JSON_STRATEGY_PATH = "nautilus_trader.trading.json_strategy:JsonStrategy"
JSON_STRATEGY_CONFIG_PATH = "nautilus_trader.trading.json_strategy:JsonStrategyConfig"


class BacktestApiRequest(msgspec.Struct, kw_only=True, frozen=True):
    """
    Represents one externally supplied backtest API request.

    Parameters
    ----------
    run_config : dict[str, Any]
        The backtest run configuration JSON object. The object is decoded into
        :class:`BacktestRunConfig` after the submitted strategies have been injected.
    strategies : list[dict[str, Any]]
        Strategy definitions owned by the caller and processed by ``JsonStrategy``.

    """

    run_config: dict[str, Any]
    strategies: list[dict[str, Any]] = []


def json_strategy_importable_config(strategy: dict[str, Any]) -> ImportableStrategyConfig:
    """
    Convert an external strategy JSON object into an importable processor config.

    Parameters
    ----------
    strategy : dict[str, Any]
        The strategy JSON supplied by the caller.

    Returns
    -------
    ImportableStrategyConfig

    """
    return ImportableStrategyConfig(
        strategy_path=JSON_STRATEGY_PATH,
        config_path=JSON_STRATEGY_CONFIG_PATH,
        config={"spec": strategy},
    )


def build_run_config_from_api_request(request: BacktestApiRequest) -> BacktestRunConfig:
    """
    Build a :class:`BacktestRunConfig` from an externally supplied API request.

    The caller's ``strategies`` are placed into ``run_config.engine.strategies``. The caller may
    still pass full engine settings; only the strategies list is replaced by the request payload.

    Parameters
    ----------
    request : BacktestApiRequest
        The external backtest request.

    Returns
    -------
    BacktestRunConfig

    """
    run_config = deepcopy(request.run_config)
    engine = dict(run_config.get("engine") or {})
    engine["strategies"] = [
        msgspec.json.decode(
            strategy_config.json(),
            type=dict[str, Any],
        )
        for strategy_config in map(json_strategy_importable_config, request.strategies)
    ]
    run_config["engine"] = engine

    raw = msgspec.json.encode(run_config, enc_hook=msgspec_encoding_hook)
    return msgspec.json.decode(
        raw,
        type=BacktestRunConfig,
        dec_hook=msgspec_decoding_hook,
    )


def parse_backtest_run_request(raw: bytes | str) -> list[BacktestRunConfig]:
    """
    Parse a JSON API request into backtest run configs.

    ``raw`` accepts either the new API shape with ``run_config`` and ``strategies`` or the legacy
    shape containing a JSON list of :class:`BacktestRunConfig` objects.

    Parameters
    ----------
    raw : bytes or str
        The JSON payload supplied by an external application.

    Returns
    -------
    list[BacktestRunConfig]

    """
    payload = msgspec.json.decode(raw)

    if isinstance(payload, dict) and "run_config" in payload:
        request = msgspec.convert(payload, type=BacktestApiRequest)
        return [build_run_config_from_api_request(request)]

    if isinstance(payload, list) and all(
        isinstance(item, dict) and "run_config" in item for item in payload
    ):
        requests = msgspec.convert(payload, type=list[BacktestApiRequest])
        return [build_run_config_from_api_request(request) for request in requests]

    return msgspec.json.decode(
        raw,
        type=list[BacktestRunConfig],
        dec_hook=msgspec_decoding_hook,
    )


def run_backtest_request(raw: bytes | str):
    """
    Run a backtest from a JSON API request.

    This is the primary callable entry point for other applications. The repository processes the
    strategy JSON provided by the caller without requiring strategy classes to be authored in files.

    Parameters
    ----------
    raw : bytes or str
        The JSON payload supplied by an external application.

    Returns
    -------
    list[BacktestResult]

    """
    configs = parse_backtest_run_request(raw)
    node = BacktestNode(configs=configs)
    return node.run()
