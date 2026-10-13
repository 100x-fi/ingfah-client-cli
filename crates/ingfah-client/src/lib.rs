//! Client for the Ingfah client API, authenticated with an API key.
//!
//! Every request resolves through [`routes::ROUTES`]; anything else is refused before it is
//! sent. No printing and no prompts here, so the crate can back other front ends.

pub mod client;
pub mod error;
pub mod routes;

pub use client::{ApiKey, Body, Client, ClientConfig, DEFAULT_BASE_URL, Request, Response};
pub use error::{ApiError, Error};
pub use routes::{Effect, Method, Route};
