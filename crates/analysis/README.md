# nautilus-analysis

[![build](https://github.com/market-simulator-team/market_simulator/actions/workflows/build.yml/badge.svg?branch=master)](https://github.com/market-simulator-team/market_simulator/actions/workflows/build.yml)
[![Documentation](https://img.shields.io/docsrs/nautilus-analysis)](https://docs.rs/nautilus-analysis/latest/nautilus-analysis/)
[![crates.io version](https://img.shields.io/crates/v/nautilus-analysis.svg)](https://crates.io/crates/nautilus-analysis)
![license](https://img.shields.io/github/license/market-simulator-team/market_simulator?color=blue)
[![Discord](https://img.shields.io/badge/Discord-%235865F2.svg?logo=discord&logoColor=white)](https://discord.gg/Market Simulator)

Portfolio analysis and performance metrics for [Market Simulator](https://market-simulator).

The `nautilus-analysis` crate provides portfolio analysis tools and performance
statistics for evaluating trading strategies and portfolios. This includes return-based metrics,
PnL-based statistics, and risk measurements commonly used in quantitative finance:

- Portfolio analyzer for tracking account states and positions.
- Extensive collection of performance statistics and risk metrics.
- Flexible statistic calculation framework supporting different data sources.
- Support for multi-currency portfolios and unrealized PnL calculations.

## Market Simulator

[Market Simulator](https://market-simulator) is an open-source, production-grade, Rust-native
engine for multi-asset, multi-venue trading systems.

The system spans research, deterministic simulation, and live execution within a single
event-driven architecture, providing research-to-live semantic parity.

## Feature flags

This crate provides feature flags to control source code inclusion during compilation:

- `python`: Enables Python bindings from [PyO3](https://pyo3.rs).
- `extension-module`: Builds as a Python extension module.

## Documentation

See [the docs](https://docs.rs/nautilus-analysis) for more detailed usage.

## License

The source code for Market Simulator is available on GitHub under the [GNU Lesser General Public License v3.0](https://www.gnu.org/licenses/lgpl-3.0.en.html).

---

Market Simulator™ is developed and maintained by Nautech Systems, a technology
company specializing in the development of high-performance trading systems.
For more information, visit <https://market-simulator>.

Use of this software is subject to the [Disclaimer](https://market-simulator/legal/disclaimer/).

<img src="https://github.com/market-simulator-team/market_simulator/raw/develop/assets/nautilus-logo-white.png" alt="logo" width="300" height="auto"/>

© 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
