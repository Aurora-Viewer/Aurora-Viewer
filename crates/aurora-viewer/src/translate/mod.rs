//! Translation of private IMs with a local model (fork experiment): the
//! partner language of each conversation, the lines being translated and
//! the order our translated messages leave in. The model and its thread are
//! in `aurora-translate`.

mod command;
mod service;
mod state;
#[cfg(test)]
mod tests;

pub use command::*;
pub use service::*;
pub use state::*;
