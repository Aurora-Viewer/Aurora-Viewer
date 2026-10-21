//! The region console of estate managers: the command lines sent to the
//! SimConsoleAsync capability and the simulator's output, in order.
//!
//! Port of LLFloaterRegionDebugConsole (indra/newview/
//! llfloaterregiondebugconsole.cpp, originally LGPL 2.1): each command is
//! echoed, its output follows when the event queue brings it, a command
//! that did not reach the simulator says so. The text is kept here, not in
//! the window, so closing the window keeps it like the Legacy Viewer's
//! floater instance does; the scrollback is bounded.

use aurora_net::NetCommand;
use std::collections::VecDeque;

/// Lines kept in the scrollback.
const MAX_LINES: usize = 2000;
/// Commands kept for the arrow keys (LLLineEditor::setEnableLineHistory).
const MAX_HISTORY: usize = 100;

/// One line of the console.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    /// A command we typed.
    Command(String),
    /// What the simulator printed.
    Output(String),
    /// The last command was not received by the simulator.
    NotSent,
}

#[derive(Default)]
pub struct SimConsole {
    lines: VecDeque<Line>,
    /// Typed commands, oldest first.
    history: Vec<String>,
    out: Vec<NetCommand>,
}

impl SimConsole {
    /// LLFloaterRegionDebugConsole::onInput.
    pub fn send(&mut self, command: &str) {
        let command = command.trim_end_matches(['\r', '\n']);
        if command.trim().is_empty() {
            return;
        }
        self.push(Line::Command(command.to_owned()));
        if self.history.last().map(String::as_str) != Some(command) {
            self.history.push(command.to_owned());
            if self.history.len() > MAX_HISTORY {
                self.history.remove(0);
            }
        }
        self.out.push(NetCommand::SimConsole(command.to_owned()));
    }

    /// The simulator's output (onReplyReceived), or the failed POST
    /// (onAsyncConsoleError).
    pub fn reply(&mut self, reply: Result<String, String>) {
        match reply {
            Ok(text) => {
                for line in text.trim_end_matches(['\r', '\n']).lines() {
                    self.push(Line::Output(line.trim_end_matches('\r').to_owned()));
                }
            }
            Err(_) => self.push(Line::NotSent),
        }
    }

    fn push(&mut self, line: Line) {
        self.lines.push_back(line);
        while self.lines.len() > MAX_LINES {
            self.lines.pop_front();
        }
    }

    pub fn lines(&self) -> impl Iterator<Item = &Line> {
        self.lines.iter()
    }

    /// The typed commands, oldest first.
    pub fn history(&self) -> &[String] {
        &self.history
    }

    pub fn take_commands(&mut self) -> Vec<NetCommand> {
        std::mem::take(&mut self.out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_is_echoed_sent_and_answered() {
        let mut c = SimConsole::default();
        c.send("help\n");
        c.send("   ");
        let sent = c.take_commands();
        assert_eq!(sent.len(), 1);
        assert!(matches!(&sent[0], NetCommand::SimConsole(s) if s == "help"));
        c.reply(Ok("commands:\r\n  help\n  stats\n\n".into()));
        c.reply(Err("status 500".into()));
        assert_eq!(
            c.lines().cloned().collect::<Vec<_>>(),
            vec![
                Line::Command("help".into()),
                Line::Output("commands:".into()),
                Line::Output("  help".into()),
                Line::Output("  stats".into()),
                Line::NotSent,
            ]
        );
    }

    #[test]
    fn the_history_skips_repeats_and_the_scrollback_is_bounded() {
        let mut c = SimConsole::default();
        c.send("stats");
        c.send("stats");
        c.send("help");
        assert_eq!(c.history(), ["stats", "help"]);
        c.reply(Ok("x\n".repeat(MAX_LINES + 10)));
        assert_eq!(c.lines().count(), MAX_LINES);
        assert_eq!(c.lines().next(), Some(&Line::Output("x".into())));
    }
}
