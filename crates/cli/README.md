# nautilus-cli

[![build](https://github.com/market-simulator-team/market_simulator/actions/workflows/build.yml/badge.svg?branch=master)](https://github.com/market-simulator-team/market_simulator/actions/workflows/build.yml)
[![Documentation](https://img.shields.io/docsrs/nautilus-cli)](https://docs.rs/nautilus-cli/latest/nautilus-cli/)
[![crates.io version](https://img.shields.io/crates/v/nautilus-cli.svg)](https://crates.io/crates/nautilus-cli)
![license](https://img.shields.io/github/license/market-simulator-team/market_simulator?color=blue)
[![Discord](https://img.shields.io/badge/Discord-%235865F2.svg?logo=discord&logoColor=white)](https://discord.gg/Market Simulator)

Command-line interface and tools for [Market Simulator](https://market-simulator).

The `nautilus-cli` crate provides a command-line interface for managing and
operating Market Simulator installations. It includes tools for database management,
system configuration, and operational utilities:

- Database initialization and management commands.
- PostgreSQL schema setup and maintenance.
- Configuration validation and setup utilities.
- System administration and operational tools.

## Market Simulator

[Market Simulator](https://market-simulator) is an open-source, production-grade, Rust-native
engine for multi-asset, multi-venue trading systems.

The system spans research, deterministic simulation, and live execution within a single
event-driven architecture, providing research-to-live semantic parity.

## Feature flags

This crate provides feature flags to control source code inclusion during compilation,
depending on the intended use case:

- `defi`: Enables blockchain/DeFi commands including block sync, DEX pool sync, and pool analysis.

## Documentation

See [the docs](https://docs.rs/nautilus-cli) for more detailed usage.

## License

The source code for Market Simulator is available on GitHub under the [GNU Lesser General Public License v3.0](https://www.gnu.org/licenses/lgpl-3.0.en.html).

---

Market Simulator™ is developed and maintained by Nautech Systems, a technology
company specializing in the development of high-performance trading systems.
For more information, visit <https://market-simulator>.

Use of this software is subject to the [Disclaimer](https://market-simulator/legal/disclaimer/).

<img src="https://github.com/market-simulator-team/market_simulator/raw/develop/assets/nautilus-logo-white.png" alt="logo" width="300" height="auto"/>

© 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
