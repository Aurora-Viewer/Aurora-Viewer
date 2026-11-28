//! Stable command identities on the ordinary menu widgets. The explicit key
//! survives translation and source edits; all action handlers stay at the caller.

use super::*;

pub struct Named<'a, K> {
    ui: &'a mut Ui,
    palette: &'a Palette,
    key: K,
}

pub fn command<'a, K: AsRef<str>>(ui: &'a mut Ui, palette: &'a Palette, key: K) -> Named<'a, K> {
    Named { ui, palette, key }
}

macro_rules! rows {
    ($($name:ident($($arg:ident: $ty:ty),*);)*) => {
        $(pub fn $name(self, $($arg: $ty),*) -> bool {
            let Self { ui, palette, key } = self;
            identified(ui, key, |ui| super::$name(ui, palette, $($arg),*))
        })*
    };
}

impl<K: AsRef<str>> Named<'_, K> {
    rows! {
        item(icon: &str, label: &str);
        item_if(icon: &str, label: &str, enabled: bool);
        item_keys(icon: &str, label: &str, keys: &str);
        item_tip(icon: &str, label: &str, enabled: bool, tip: &str);
        item_detail(icon: &str, label: &str, detail: &str);
        check(icon: &str, label: &str, checked: bool, enabled: bool);
        check_keys(icon: &str, label: &str, checked: bool, enabled: bool, keys: &str);
        toggle(icon: &str, label: &str, on: &mut bool);
        toolbar_command(id: &'static str);
        toolbar_command_keys(id: &'static str, keys: &str);
        toolbar_command_keys_if(id: &'static str, label: &str, enabled: bool, keys: &str);
        toolbar_command_check(id: &'static str, checked: bool);
        toolbar_command_tip(id: &'static str, enabled: bool, tip: &str);
    }

    pub fn submenu(self, icon: &str, label: &str, enabled: bool, body: impl FnOnce(&mut Ui)) {
        let Self { ui, palette, key } = self;
        identified(ui, key, |ui| super::submenu(ui, palette, icon, label, enabled, body));
    }
}
