//! pacod-broker: a small, fast broker where providers announce AI and compute resources (models,
//! MCP servers, agents, tools, GPUs) with their prices, availability and interfaces over time,
//! and consumers discover them, with reputation for both sides (docs/concept.md).

pub mod api;
pub mod app;
pub mod auth;
pub mod config;
pub mod env;
pub mod feeds;
pub mod identity;
pub mod metrics;
pub mod openapi;
pub mod registry;
pub mod server;
pub mod state;
pub mod tickets;
