//! Allowlisted debug writes from RlvExtGetSet (rlvextensions.cpp, originally
//! LGPL 2.1). Overrides stay session-local and respect other issuers' locks.
use super::{Command, Rlva};
use uuid::Uuid;

impl Rlva {
    pub fn debug_force(&mut self, actor: Uuid, command: &Command) -> bool {
        if !matches!(
            command.behaviour.as_str(),
            "setdebug_avatarsex" | "setdebug_renderresolutiondivisor"
        ) {
            return false;
        }
        if !self.enabled() || command.param != super::Param::Force || self.has_except("setdebug", actor) {
            return true;
        }
        match command.behaviour.as_str() {
            "setdebug_avatarsex" => match command.option.as_str() {
                "true" | "1" => self.pseudo_sex = Some(true),
                "false" | "0" => self.pseudo_sex = Some(false),
                "default" => self.pseudo_sex = None,
                _ => {}
            },
            _ => {
                if command.option == "default" {
                    self.resolution_divisor = None;
                } else if let Ok(value) = command.option.parse::<u32>() {
                    self.resolution_divisor = Some(value);
                }
            }
        }
        true
    }
}
