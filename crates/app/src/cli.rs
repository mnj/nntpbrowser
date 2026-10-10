//! Headless front end to the same engine used by the desktop browser.

mod parser;
mod runner;

pub use parser::{parse_cli, CliCommand};
pub use runner::run_cli;

#[cfg(test)]
mod tests;
