// -------------------------------------------------------------------------------------------------
//  Copyright (C) 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
//  https://nautechsystems.io
//
//  Licensed under the GNU Lesser General Public License Version 3.0 (the "License");
//  You may not use this file except in compliance with the License.
//  You may obtain a copy of the License at https://www.gnu.org/licenses/lgpl-3.0.en.html
//
//  Unless required by applicable law or agreed to in writing, software
//  distributed under the License is distributed on an "AS IS" BASIS,
//  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//  See the License for the specific language governing permissions and
//  limitations under the License.
// -------------------------------------------------------------------------------------------------

//! Embeddable simulation SDK for external applications.
//!
//! This module lets a host application drive the [`crate::engine::BacktestEngine`] as a pure
//! processing engine:
//!
//! - **The caller owns all data.** Bars are handed in as plain `Vec<Bar>` batches; nothing is
//!   read from disk or any catalog, and all engine state is dropped when the run completes.
//! - **The caller owns the strategy logic.** [`CallbackStrategy`] adapts a host-provided
//!   closure into the engine's strategy machinery, so strategy definitions never need to live
//!   in this repository.
//! - **Per-asset-class venue presets.** [`VenuePreset`] configures the simulated exchange with
//!   account type, book type, and leverage appropriate to each asset class so every class runs
//!   on the engine configuration best suited to it.
//! - **Live progress and cooperative cancellation.** [`SimulationControl`] exposes atomics the
//!   host can poll from another thread while the run executes in chunked streaming mode.

use std::{
    fmt::Debug,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use nautilus_common::actor::DataActor;
use nautilus_model::{
    data::{Bar, BarType, Data},
    enums::{AccountType, BookType, CurrencyType, OmsType, OrderSide},
    identifiers::{InstrumentId, StrategyId, Symbol, Venue},
    instruments::{CurrencyPair, Equity, Instrument, InstrumentAny},
    types::{Currency, Money, Price, Quantity},
};
use nautilus_trading::{
    nautilus_strategy,
    strategy::{Strategy, StrategyConfig, StrategyCore},
};
use rust_decimal::Decimal;

use crate::{
    config::{BacktestEngineConfig, SimulatedVenueConfig},
    engine::BacktestEngine,
    result::BacktestResult,
};

// ── Progress and cancellation ────────────────────────────────────────────────

/// Shared control block for a running simulation.
///
/// The host application keeps a clone of the [`Arc`] and polls
/// [`progress`](Self::progress) for UI updates or flips
/// [`cancel`](Self::cancel) to stop the run at the next chunk boundary.
#[derive(Debug, Default)]
pub struct SimulationControl {
    processed: AtomicU64,
    total: AtomicU64,
    cancelled: AtomicBool,
}

impl SimulationControl {
    /// Creates a new control block wrapped in an [`Arc`].
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Requests cooperative cancellation at the next chunk boundary.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    /// Whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    /// Number of data events processed so far.
    #[must_use]
    pub fn processed(&self) -> u64 {
        self.processed.load(Ordering::Relaxed)
    }

    /// Total number of data events in the run.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }

    /// Completion ratio in `[0.0, 1.0]` (0.0 until the total is known).
    #[must_use]
    pub fn progress(&self) -> f64 {
        let total = self.total();
        if total == 0 {
            return 0.0;
        }
        self.processed() as f64 / total as f64
    }

    fn set_total(&self, total: u64) {
        self.total.store(total, Ordering::Relaxed);
    }

    fn add_processed(&self, n: u64) {
        self.processed.fetch_add(n, Ordering::Relaxed);
    }
}

// ── Caller-owned strategy adapter ────────────────────────────────────────────

/// An order instruction returned by a host bar handler.
#[derive(Debug, Clone)]
pub enum SimOrderCommand {
    /// Submit a market order on the simulated venue.
    Market {
        /// Order side.
        side: OrderSide,
        /// Order quantity (validated against the instrument by the engine).
        quantity: Quantity,
    },
}

/// Host-provided bar handler: receives each bar and returns order commands.
pub type BarHandler = Box<dyn FnMut(&Bar) -> Vec<SimOrderCommand> + Send>;

/// Adapts a host-owned closure into an engine strategy.
///
/// The closure receives every bar for the subscribed bar types and returns
/// [`SimOrderCommand`]s which are routed through the engine's order factory
/// and execution pipeline. All trading decisions therefore remain with the
/// caller; this type only owns the engine plumbing.
pub struct CallbackStrategy {
    core: StrategyCore,
    instrument_id: InstrumentId,
    bar_types: Vec<BarType>,
    handler: BarHandler,
}

impl CallbackStrategy {
    /// Creates a new [`CallbackStrategy`] for `instrument_id`, subscribing to `bar_types`.
    #[must_use]
    pub fn new(instrument_id: InstrumentId, bar_types: Vec<BarType>, handler: BarHandler) -> Self {
        let config = StrategyConfig {
            strategy_id: Some(StrategyId::from("CALLBACK-001")),
            order_id_tag: Some("001".to_string()),
            ..Default::default()
        };
        Self {
            core: StrategyCore::new(config),
            instrument_id,
            bar_types,
            handler,
        }
    }
}

nautilus_strategy!(CallbackStrategy);

impl Debug for CallbackStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct(stringify!(CallbackStrategy))
            .field("instrument_id", &self.instrument_id)
            .field("bar_types", &self.bar_types)
            .finish_non_exhaustive()
    }
}

impl DataActor for CallbackStrategy {
    fn on_start(&mut self) -> anyhow::Result<()> {
        for bar_type in self.bar_types.clone() {
            self.subscribe_bars(bar_type, None, None);
        }
        Ok(())
    }

    fn on_bar(&mut self, bar: &Bar) -> anyhow::Result<()> {
        let commands = (self.handler)(bar);
        for command in commands {
            match command {
                SimOrderCommand::Market { side, quantity } => {
                    let order = self.core.order_factory().market(
                        self.instrument_id,
                        side,
                        quantity,
                        None, // time_in_force
                        None, // reduce_only
                        None, // quote_quantity
                        None, // display_qty
                        None, // expire_time
                        None, // emulation_trigger
                        None, // tags
                    );
                    self.submit_order(order, None, None, None)?;
                }
            }
        }
        Ok(())
    }
}

// ── Per-asset-class venue presets ────────────────────────────────────────────

/// Simulated-venue presets tuned per asset class.
///
/// Each preset selects the account type, order-management model, book type,
/// and leverage that best matches how the asset class trades, so the host
/// application can run every class on an appropriately configured engine
/// without knowing the engine internals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VenuePreset {
    /// Centralized-exchange crypto spot (cash account, top-of-book CLOB).
    CryptoSpot,
    /// Crypto perpetual swaps (margin account, 10x default leverage).
    CryptoPerpetual,
    /// Cash equities and ETFs (cash account, broker-style top-of-book).
    Equity,
    /// Spot FX (margin account, 50x default leverage).
    Fx,
    /// Expiring futures (margin account, 20x default leverage).
    Futures,
    /// Listed options (cash account).
    Options,
}

impl VenuePreset {
    /// Resolves a preset from a free-form asset-class string.
    ///
    /// Accepts the snake_case class keys used by typical host applications
    /// (e.g. `"crypto_spot_cex"`, `"equity"`, `"perpetual_swap"`, `"fx"`).
    /// Unknown values fall back to [`VenuePreset::CryptoSpot`], the most
    /// permissive 24/7 cash configuration.
    #[must_use]
    pub fn from_asset_class(asset_class: &str) -> Self {
        match asset_class {
            "equity" | "etf" | "bond" => Self::Equity,
            "perpetual_swap" => Self::CryptoPerpetual,
            "fx" => Self::Fx,
            "futures_expiring" => Self::Futures,
            "option" => Self::Options,
            _ => Self::CryptoSpot,
        }
    }

    /// Builds the [`SimulatedVenueConfig`] for this preset.
    #[must_use]
    pub fn venue_config(
        self,
        venue: Venue,
        starting_balances: Vec<Money>,
    ) -> SimulatedVenueConfig {
        let builder = SimulatedVenueConfig::builder()
            .venue(venue)
            .oms_type(OmsType::Netting)
            .book_type(BookType::L1_MBP)
            .starting_balances(starting_balances)
            .bar_execution(true);

        match self {
            Self::CryptoSpot | Self::Options => {
                builder.account_type(AccountType::Cash).build()
            }
            Self::Equity => builder
                .account_type(AccountType::Cash)
                .reject_stop_orders(false)
                .build(),
            Self::CryptoPerpetual => builder
                .account_type(AccountType::Margin)
                .default_leverage(Decimal::from(10))
                .build(),
            Self::Fx => builder
                .account_type(AccountType::Margin)
                .default_leverage(Decimal::from(50))
                .build(),
            Self::Futures => builder
                .account_type(AccountType::Margin)
                .default_leverage(Decimal::from(20))
                .build(),
        }
    }
}

// ── Instrument helpers ───────────────────────────────────────────────────────

/// Returns a registered [`Currency`] for `code`, registering a crypto
/// currency with `precision` decimal places when the code is unknown.
#[must_use]
pub fn currency_or_register(code: &str, precision: u8) -> Currency {
    if let Some(currency) = Currency::try_from_str(code) {
        return currency;
    }
    let currency = Currency::new(code, precision, 0, code, CurrencyType::Crypto);
    // Ignore the result: a concurrent registration of the same code is fine.
    let _ = Currency::register(currency, false);
    currency
}

/// Builds a spot pair instrument (crypto spot or FX) for the simulation.
///
/// `symbol` is the venue symbol (e.g. `"BTC-USDT"`), and `base`/`quote` are
/// the currency codes on each side of the pair.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn spot_pair_instrument(
    venue: Venue,
    symbol: &str,
    base: &str,
    quote: &str,
    price_precision: u8,
    size_precision: u8,
    price_increment: Price,
    size_increment: Quantity,
) -> InstrumentAny {
    let raw_symbol = Symbol::from(symbol);
    let instrument_id = InstrumentId::new(raw_symbol, venue);
    InstrumentAny::CurrencyPair(CurrencyPair::new(
        instrument_id,
        raw_symbol,
        currency_or_register(base, size_precision),
        currency_or_register(quote, price_precision),
        price_precision,
        size_precision,
        price_increment,
        size_increment,
        None, // multiplier
        None, // lot_size
        None, // max_quantity
        None, // min_quantity
        None, // max_notional
        None, // min_notional
        None, // max_price
        None, // min_price
        None, // margin_init
        None, // margin_maint
        None, // maker_fee
        None, // taker_fee
        None, // info
        Default::default(),
        Default::default(),
    ))
}

/// Builds a cash equity instrument for the simulation.
#[must_use]
pub fn equity_instrument(
    venue: Venue,
    symbol: &str,
    quote: &str,
    price_precision: u8,
    price_increment: Price,
) -> InstrumentAny {
    let raw_symbol = Symbol::from(symbol);
    let instrument_id = InstrumentId::new(raw_symbol, venue);
    InstrumentAny::Equity(Equity::new(
        instrument_id,
        raw_symbol,
        None, // isin
        currency_or_register(quote, price_precision),
        price_precision,
        price_increment,
        None, // lot_size
        None, // max_quantity
        None, // min_quantity
        None, // max_price
        None, // min_price
        None, // margin_init
        None, // margin_maint
        None, // maker_fee
        None, // taker_fee
        None, // info
        Default::default(),
        Default::default(),
    ))
}

// ── Bar-driven simulation runner ─────────────────────────────────────────────

/// Specification for one bar-driven simulation run.
#[derive(Debug)]
pub struct BarSimulationSpec {
    /// Simulated venue identifier (e.g. `Venue::from("KRAKEN")`).
    pub venue: Venue,
    /// Asset-class venue preset.
    pub preset: VenuePreset,
    /// Instrument to simulate (see [`spot_pair_instrument`] / [`equity_instrument`]).
    pub instrument: InstrumentAny,
    /// Starting account balances (e.g. `vec![Money::from("100_000 USD")]`).
    pub starting_balances: Vec<Money>,
    /// Bar types the strategy subscribes to.
    pub bar_types: Vec<BarType>,
    /// Number of bars fed per streaming chunk (progress granularity).
    pub chunk_size: usize,
}

/// Outcome of a bar-driven simulation run.
#[derive(Debug)]
pub struct SimulationOutcome {
    /// Final engine results (orders, positions, PnL and return statistics).
    pub result: BacktestResult,
    /// Whether the run stopped early due to a cancellation request.
    pub cancelled: bool,
}

/// Runs a bar-driven simulation to completion (or cancellation).
///
/// Data is processed in chunks of `spec.chunk_size` bars using the engine's
/// streaming mode; after every chunk the shared `control` block is updated
/// with progress and checked for cancellation. All data and engine state are
/// owned by this function and dropped on return — nothing is persisted.
///
/// # Errors
///
/// Returns an error if the engine rejects the configuration, instrument,
/// data, or fails during the run.
pub fn run_bar_simulation(
    spec: BarSimulationSpec,
    bars: Vec<Bar>,
    handler: BarHandler,
    control: &Arc<SimulationControl>,
) -> anyhow::Result<SimulationOutcome> {
    anyhow::ensure!(!bars.is_empty(), "no bars supplied for simulation");
    let chunk_size = spec.chunk_size.max(1);
    control.set_total(bars.len() as u64);

    let engine_config = BacktestEngineConfig::builder()
        .logging(
            nautilus_common::logging::logger::LoggerConfig {
                stdout_level: log::LevelFilter::Warn,
                ..Default::default()
            },
        )
        .build();
    let mut engine = BacktestEngine::new(engine_config)?;

    engine.add_venue(spec.preset.venue_config(spec.venue, spec.starting_balances))?;
    engine.add_instrument(&spec.instrument)?;

    let instrument_id = spec.instrument.id();
    engine.add_strategy(CallbackStrategy::new(instrument_id, spec.bar_types, handler))?;

    let mut cancelled = false;
    let mut chunks = bars.into_iter().peekable();
    let mut first = true;

    while chunks.peek().is_some() {
        if control.is_cancelled() {
            cancelled = true;
            break;
        }

        let batch: Vec<Data> = chunks.by_ref().take(chunk_size).map(Data::Bar).collect();
        let batch_len = batch.len() as u64;

        if !first {
            engine.clear_data();
        }
        engine.add_data(batch, None, true, true)?;
        engine.run(None, None, None, true)?;
        first = false;

        control.add_processed(batch_len);
    }

    engine.end();
    let result = engine.get_result();

    Ok(SimulationOutcome { result, cancelled })
}

#[cfg(test)]
mod tests {
    use nautilus_core::UnixNanos;
    use nautilus_model::data::BarSpecification;
    use nautilus_model::enums::{AggregationSource, BarAggregation, PriceType};

    use super::*;

    fn bar(bar_type: BarType, mid: f64, ts: u64) -> Bar {
        Bar::new(
            bar_type,
            Price::from(format!("{:.2}", mid - 0.5).as_str()),
            Price::from(format!("{:.2}", mid + 1.0).as_str()),
            Price::from(format!("{:.2}", mid - 1.0).as_str()),
            Price::from(format!("{mid:.2}").as_str()),
            // Volume precision must match the instrument size precision or the
            // matching engine skips the bar for execution.
            Quantity::from("100.000000"),
            UnixNanos::from(ts),
            UnixNanos::from(ts),
        )
    }

    fn test_spec() -> (BarSimulationSpec, BarType) {
        let venue = Venue::from("SIM");
        let instrument = spot_pair_instrument(
            venue,
            "BTC-USDT",
            "BTC",
            "USDT",
            2,
            6,
            Price::from("0.01"),
            Quantity::from("0.000001"),
        );
        let bar_type = BarType::new(
            instrument.id(),
            BarSpecification::new(1, BarAggregation::Minute, PriceType::Last),
            AggregationSource::External,
        );
        let spec = BarSimulationSpec {
            venue,
            preset: VenuePreset::CryptoSpot,
            instrument,
            starting_balances: vec![Money::from("1_000_000 USDT")],
            bar_types: vec![bar_type],
            chunk_size: 16,
        };
        (spec, bar_type)
    }

    fn synthetic_bars(bar_type: BarType, count: usize) -> Vec<Bar> {
        let base_ts: u64 = 1_735_689_600_000_000_000; // 2025-01-01T00:00:00Z
        let minute: u64 = 60_000_000_000;
        (0..count)
            .map(|i| {
                let mid = 50_000.0 + (i as f64 * 10.0);
                bar(bar_type, mid, base_ts + i as u64 * minute)
            })
            .collect()
    }

    #[test]
    fn callback_strategy_orders_reach_the_venue() {
        let (spec, bar_type) = test_spec();
        let bars = synthetic_bars(bar_type, 64);
        let control = SimulationControl::new();

        let mut seen = 0usize;
        let handler: BarHandler = Box::new(move |_bar| {
            seen += 1;
            if seen == 10 {
                vec![SimOrderCommand::Market {
                    side: OrderSide::Buy,
                    // Quantity precision must match the instrument size precision.
                    quantity: Quantity::from("0.010000"),
                }]
            } else {
                vec![]
            }
        });

        let outcome = run_bar_simulation(spec, bars, handler, &control).unwrap();

        assert!(!outcome.cancelled);
        assert_eq!(outcome.result.total_orders, 1);
        assert_eq!(outcome.result.total_positions, 1, "order should have filled");
        assert_eq!(outcome.result.iterations, 64);
        assert_eq!(control.processed(), 64);
        assert!((control.progress() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn cancellation_stops_at_chunk_boundary() {
        let (spec, bar_type) = test_spec();
        let bars = synthetic_bars(bar_type, 64);
        let control = SimulationControl::new();

        let cancel_handle = Arc::clone(&control);
        let mut seen = 0usize;
        let handler: BarHandler = Box::new(move |_bar| {
            seen += 1;
            if seen == 8 {
                cancel_handle.cancel();
            }
            vec![]
        });

        let outcome = run_bar_simulation(spec, bars, handler, &control).unwrap();

        assert!(outcome.cancelled);
        assert!(control.processed() < 64);
    }

    #[test]
    fn venue_presets_cover_asset_classes() {
        assert_eq!(
            VenuePreset::from_asset_class("crypto_spot_cex"),
            VenuePreset::CryptoSpot
        );
        assert_eq!(VenuePreset::from_asset_class("equity"), VenuePreset::Equity);
        assert_eq!(
            VenuePreset::from_asset_class("perpetual_swap"),
            VenuePreset::CryptoPerpetual
        );
        assert_eq!(VenuePreset::from_asset_class("fx"), VenuePreset::Fx);
        assert_eq!(
            VenuePreset::from_asset_class("futures_expiring"),
            VenuePreset::Futures
        );
        assert_eq!(VenuePreset::from_asset_class("option"), VenuePreset::Options);
        assert_eq!(
            VenuePreset::from_asset_class("unknown"),
            VenuePreset::CryptoSpot
        );
    }
}
