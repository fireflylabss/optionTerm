//! GTK-dependent halves of [`option_term_core::keys::Bindings`]: applying the
//! accelerators to a `GtkApplication` and reflecting them in tooltips.

use gtk4::prelude::*;
use option_term_core::{commands::COMMANDS, keys::Bindings};

pub trait BindingsGtk {
    fn apply(&self, app: &impl IsA<gtk4::Application>);
    fn update_tooltips(&self, widget: &gtk4::Widget);
}

impl BindingsGtk for Bindings {
    fn apply(&self, app: &impl IsA<gtk4::Application>) {
        for (_, action, _) in COMMANDS {
            let name = action.trim_start_matches("win.");
            let mut accels = self.accels(name);
            if accels.iter().any(|a| gtk4::accelerator_parse(a).is_none()) {
                tracing::warn!(action = name, "invalid shortcut; restoring default");
                accels = Self::default().accels(name);
            }
            app.set_accels_for_action(
                action,
                &accels.iter().map(String::as_str).collect::<Vec<_>>(),
            );
        }
    }

    fn update_tooltips(&self, widget: &gtk4::Widget) {
        if let Some(actionable) = widget.dynamic_cast_ref::<gtk4::Actionable>()
            && let Some(action) = actionable.action_name()
            && let Some((label, _, _)) = COMMANDS.iter().find(|(_, name, _)| *name == action)
        {
            let shortcut = self.display(action.trim_start_matches("win."));
            let text = if shortcut.is_empty() {
                label.to_string()
            } else {
                format!("{label} ({shortcut})")
            };
            widget.set_tooltip_text(Some(&text));
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            self.update_tooltips(&widget);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gtk4::test]
    fn reset_reinstalls_defaults_and_aliases_immediately() {
        let app = gtk4::Application::new(
            Some("io.option.keys.test"),
            gtk4::gio::ApplicationFlags::NON_UNIQUE,
        );
        let mut bindings = Bindings::default();
        bindings.set("next-tab", Some("<Control>j"));
        bindings.apply(&app);
        assert_eq!(app.accels_for_action("win.next-tab").len(), 1);
        bindings.set("next-tab", None);
        bindings.apply(&app);
        assert_eq!(app.accels_for_action("win.next-tab").len(), 2);
        bindings.set("next-tab", Some(""));
        bindings.apply(&app);
        assert!(app.accels_for_action("win.next-tab").is_empty());
    }

    #[gtk4::test]
    fn regression_catalog_keys_are_valid_gtk_accelerators() {
        use option_term_core::keys::to_gtk_accel;
        for (_, _, accel) in COMMANDS {
            if !accel.is_empty() {
                assert!(
                    gtk4::accelerator_parse(to_gtk_accel(accel)).is_some(),
                    "invalid shortcut {accel}"
                );
            }
        }
    }
}
