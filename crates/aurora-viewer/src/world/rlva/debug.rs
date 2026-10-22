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
                "true" | "1" | "false" | "0" => self.pseudo_sex = Some(command.option.clone()),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rlva_avatar_sex_pseudo_debug_retains_the_original_boolean_text() {
        let mut r = Rlva::default();
        r.configure(true);
        for text in ["true", "false", "1", "0"] {
            r.debug_force(
                Uuid::from_u128(1),
                &Command::parse(&format!("setdebug_avatarsex:{text}=force")).unwrap(),
            );
            assert_eq!(r.pseudo_sex.as_deref(), Some(text));
        }
        r.debug_force(Uuid::from_u128(1), &Command::parse("setdebug_avatarsex:default=force").unwrap());
        assert_eq!(r.pseudo_sex.as_deref(), Some("0"), "default is not a valid pseudo boolean");
        r.configure(false);
        assert!(r.pseudo_sex.is_none());
    }
}
