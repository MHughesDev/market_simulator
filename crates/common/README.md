# nautilus-common

[![build](https://github.com/market-simulator-team/market_simulator/actions/workflows/build.yml/badge.svg?branch=master)](https://github.com/market-simulator-team/market_simulator/actions/workflows/build.yml)
[![Documentation](https://img.shields.io/docsrs/nautilus-common)](https://docs.rs/nautilus-common/latest/nautilus-common/)
[![crates.io version](https://img.shields.io/crates/v/nautilus-common.svg)](https://crates.io/crates/nautilus-common)
![license](https://img.shields.io/github/license/market-simulator-team/market_simulator?color=blue)
[![Discord](https://img.shields.io/badge/Discord-%235865F2.svg?logo=discord&logoColor=white)](https://discord.gg/Market Simulator)

Common componentry for [Market Simulator](https://market-simulator).

The `nautilus-common` crate provides shared components and utilities that form the system foundation for
Market Simulator applications. This includes the actor system, message bus, caching layer, and other
essential services.

## Market Simulator

[Market Simulator](https://market-simulator) is an open-source, production-grade, Rust-native
engine for multi-asset, multi-venue trading systems.

The system spans research, deterministic simulation, and live execution within a single
event-driven architecture, providing research-to-live semantic parity.

## Feature flags

This crate provides feature flags to control source code inclusion during compilation:

- `ffi`: Enables the C foreign function interface (FFI) from [cbindgen](https://github.com/mozilla/cbindgen).
- `python`: Enables Python bindings from [PyO3](https://pyo3.rs).
- `defi`: Enables DeFi (Decentralized Finance) support.
- `indicators`: Includes the `nautilus-indicators` crate and indicator utilities.
- `capnp`: Enables [Cap'n Proto](https://capnproto.org/) serialization support.
- `live`: Enables the Tokio async runtime for live trading.
- `tracing-bridge`: Enables the `tracing` subscriber bridge for log integration.
- `extension-module`: Builds as a Python extension module.

## Documentation

See [the docs](https://docs.rs/nautilus-common) for more detailed usage.

## License

The source code for Market Simulator is available on GitHub under the [GNU Lesser General Public License v3.0](https://www.gnu.org/licenses/lgpl-3.0.en.html).

---

Market Simulator™ is developed and maintained by Nautech Systems, a technology
company specializing in the development of high-performance trading systems.
For more information, visit <https://market-simulator>.

Use of this software is subject to the [Disclaimer](https://market-simulator/legal/disclaimer/).

<img src="https://github.com/market-simulator-team/market_simulator/raw/develop/assets/nautilus-logo-white.png" alt="logo" width="300" height="auto"/>

© 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
