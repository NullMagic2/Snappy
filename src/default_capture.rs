//! Module: default_capture. Purpose: reversible per-user Print Screen desktop integration.
//! Created: 2026-10-19. Architecture: installation owns settings; capture never changes them.
use crate::{desktop, files::Result};
use glib::{Variant, variant::ToVariant};
use gtk::{gio, prelude::*};
use std::path::PathBuf;

struct Desktop {
    parent: &'static str,
    list: &'static str,
    custom: &'static str,
    prefix: &'static str,
    entry: &'static str,
    builtins: &'static [&'static str],
    array_binding: bool,
}

impl Desktop {
    fn current() -> Result<Self> {
        let session = std::env::var("XDG_CURRENT_DESKTOP")
            .unwrap_or_default()
            .to_lowercase();
        if session.split(':').any(|part| part.contains("cinnamon")) {
            Ok(Self {
                parent: "org.cinnamon.desktop.keybindings",
                list: "custom-list",
                custom: "org.cinnamon.desktop.keybindings.custom-keybinding",
                prefix: "/org/cinnamon/desktop/keybindings/custom-keybindings/",
                entry: "snappy",
                builtins: &[
                    "org.cinnamon.desktop.keybindings.media-keys",
                    "org.cinnamon.desktop.keybindings",
                ],
                array_binding: true,
            })
        } else if session
            .split(':')
            .any(|part| part == "gnome" || part == "ubuntu")
        {
            Ok(Self {
                parent: "org.gnome.settings-daemon.plugins.media-keys",
                list: "custom-keybindings",
                custom: "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding",
                prefix: "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/",
                entry: "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/snappy/",
                builtins: &[
                    "org.gnome.shell.keybindings",
                    "org.gnome.settings-daemon.plugins.media-keys",
                ],
                array_binding: false,
            })
        } else {
            Err("Automatic Print Screen setup supports GNOME and Cinnamon. Bind Print Screen to snappy --capture in your desktop's Keyboard Shortcuts, or install with --no-default.".into())
        }
    }

    fn path(&self, entry: &str) -> Result<String> {
        if self.array_binding {
            if entry.is_empty()
                || !entry
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                return Err("Invalid Cinnamon shortcut identifier".into());
            }
            Ok(format!("{}{entry}/", self.prefix))
        } else if entry.starts_with(self.prefix) && entry.ends_with('/') {
            Ok(entry.to_owned())
        } else {
            Err("Invalid GNOME shortcut path".into())
        }
    }
}

fn settings(id: &str, path: Option<&str>) -> Result<gio::Settings> {
    let schema = gio::SettingsSchemaSource::default()
        .and_then(|source| source.lookup(id, true))
        .ok_or_else(|| format!("Desktop settings schema {id} is unavailable"))?;
    Ok(gio::Settings::new_full(
        &schema,
        gio::SettingsBackend::NONE,
        path,
    ))
}

struct Change {
    settings: gio::Settings,
    key: String,
    value: Variant,
    list_entry: Option<String>,
}

/// Validate all writes before backing up and changing any desktop preference.
fn change(
    plan: &mut Vec<Change>,
    settings: &gio::Settings,
    key: &str,
    value: Variant,
) -> Result<()> {
    let schema = settings
        .settings_schema()
        .ok_or("Missing settings schema")?;
    if !schema.has_key(key) || !schema.key(key).range_check(&value) || !settings.is_writable(key) {
        return Err(format!(
            "Desktop preference {}:{key} cannot be changed",
            schema.id()
        ));
    }
    plan.push(Change {
        settings: settings.clone(),
        key: key.into(),
        value,
        list_entry: None,
    });
    Ok(())
}

fn remove_print(plan: &mut Vec<Change>, settings: &gio::Settings, key: &str) -> Result<()> {
    let value = settings.value(key);
    let next = if let Some(bindings) = value.get::<Vec<String>>() {
        let filtered: Vec<_> = bindings
            .iter()
            .filter(|b| !b.eq_ignore_ascii_case("Print"))
            .cloned()
            .collect();
        (filtered != bindings).then(|| filtered.to_variant())
    } else {
        value
            .get::<String>()
            .filter(|b| b.eq_ignore_ascii_case("Print"))
            .map(|_| "".to_variant())
    };
    if let Some(value) = next {
        change(plan, settings, key, value)?;
    }
    Ok(())
}

fn backup_path() -> PathBuf {
    glib::user_config_dir().join("snappy/print-screen-backup.ini")
}

/// Install the desktop-owned shortcut so it works even when Snappy is not running.
pub fn install() -> Result<()> {
    let desktop = Desktop::current()?;
    let parent = settings(desktop.parent, None)?;
    let path = desktop.path(desktop.entry)?;
    let custom = settings(desktop.custom, Some(&path))?;
    let mut entries: Vec<String> = parent
        .strv(desktop.list)
        .iter()
        .map(|s| s.to_string())
        .collect();
    let mut plan = Vec::new();
    for id in desktop.builtins {
        // Some old GNOME releases have no Shell screenshot schema.
        if let Ok(builtins) = settings(id, None) {
            for key in builtins
                .settings_schema()
                .ok_or("Missing settings schema")?
                .list_keys()
            {
                if key.as_str() != desktop.list {
                    remove_print(&mut plan, &builtins, &key)?;
                }
            }
        }
    }
    for entry in &entries {
        if entry == desktop.entry || entry == "__dummy__" {
            continue;
        }
        let other = settings(desktop.custom, Some(&desktop.path(entry)?))?;
        remove_print(&mut plan, &other, "binding")?;
    }
    change(&mut plan, &custom, "name", "Snappy".to_variant())?;
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let quoted = glib::shell_quote(executable);
    let command = format!(
        "{} --capture",
        quoted
            .to_str()
            .ok_or("Shortcut executable path must be UTF-8")?
    );
    change(&mut plan, &custom, "command", command.to_variant())?;
    let binding = if desktop.array_binding {
        vec!["Print"].to_variant()
    } else {
        "Print".to_variant()
    };
    change(&mut plan, &custom, "binding", binding)?;
    if !entries.iter().any(|e| e == desktop.entry) {
        entries.push(desktop.entry.into());
    }
    change(&mut plan, &parent, desktop.list, entries.to_variant())?;
    plan.last_mut().unwrap().list_entry = Some(desktop.entry.into());

    let backup = glib::KeyFile::new();
    let path = backup_path();
    if path.exists() {
        backup
            .load_from_file(&path, glib::KeyFileFlags::NONE)
            .map_err(|e| e.to_string())?;
    }
    let previous: Vec<_> = plan.iter().map(|c| c.settings.user_value(&c.key)).collect();
    for c in &plan {
        let schema = c.settings.schema_id().ok_or("Missing schema ID")?;
        let setting_path = c.settings.path().ok_or("Missing settings path")?;
        let group = format!("{schema}:{setting_path}:{}", c.key);
        if !backup.has_group(&group) {
            backup.set_string(&group, "schema", &schema);
            backup.set_string(&group, "path", &setting_path);
            backup.set_string(&group, "key", &c.key);
            let before = c.settings.user_value(&c.key);
            backup.set_boolean(&group, "had-user-value", before.is_some());
            backup.set_string(
                &group,
                "before",
                &before
                    .unwrap_or_else(|| c.settings.value(&c.key))
                    .print(true),
            );
        }
        backup.set_string(&group, "after", &c.value.print(true));
        if let Some(entry) = &c.list_entry {
            backup.set_string(&group, "list-entry", entry);
        }
    }
    desktop::write_file(&path, backup.to_data().as_bytes(), true)?;
    for c in &plan {
        if let Err(error) = c.settings.set_value(&c.key, &c.value) {
            for (c, old) in plan.iter().zip(&previous) {
                if let Some(value) = old {
                    let _ = c.settings.set_value(&c.key, value);
                } else {
                    c.settings.reset(&c.key);
                }
            }
            gio::Settings::sync();
            return Err(format!(
                "Could not assign Print Screen: {error}. Previous settings were restored."
            ));
        }
    }
    gio::Settings::sync();
    Ok(())
}

/// Restore only preferences still owned by Snappy, preserving later user changes.
pub fn restore() -> Result<()> {
    let path = backup_path();
    if !path.exists() {
        return Err("No saved Print Screen settings to restore".into());
    }
    let backup = glib::KeyFile::new();
    backup
        .load_from_file(&path, glib::KeyFileFlags::NONE)
        .map_err(|e| e.to_string())?;
    let mut conflicts = Vec::new();
    for group in backup.groups().iter().rev() {
        let read = |key| backup.string(group, key).map_err(|e| e.to_string());
        let id = read("schema")?;
        let setting_path = read("path")?;
        let key = read("key")?;
        let target = settings(&id, Some(&setting_path))?;
        let after = Variant::parse(None, &read("after")?).map_err(|e| e.to_string())?;
        let before = Variant::parse(None, &read("before")?).map_err(|e| e.to_string())?;
        let current = target.value(&key);
        if !target.is_writable(&key) {
            conflicts.push(group.to_string());
            continue;
        }
        if current == after {
            if backup
                .boolean(group, "had-user-value")
                .map_err(|e| e.to_string())?
            {
                target.set_value(&key, &before).map_err(|e| e.to_string())?;
            } else {
                target.reset(&key);
            }
        } else if let Ok(entry) = read("list-entry") {
            let original = before
                .get::<Vec<String>>()
                .ok_or("Invalid shortcut-list backup")?;
            if !original.contains(&entry.to_string()) {
                let mut current = current
                    .get::<Vec<String>>()
                    .ok_or("Invalid shortcut list")?;
                current.retain(|s| s != entry.as_str());
                target
                    .set_value(&key, &current.to_variant())
                    .map_err(|e| e.to_string())?;
            }
        } else if current != before {
            conflicts.push(group.to_string());
            continue;
        }
        backup.remove_group(group).map_err(|e| e.to_string())?;
    }
    gio::Settings::sync();
    if backup.groups().is_empty() {
        std::fs::remove_file(path).map_err(|e| e.to_string())?;
    } else {
        desktop::write_file(&path, backup.to_data().as_bytes(), true)?;
    }
    if !conflicts.is_empty() {
        return Err(format!(
            "Kept {} preferences changed since installation; remaining backup: {}",
            conflicts.len(),
            path_display()
        ));
    }
    Ok(())
}

fn path_display() -> String {
    backup_path().display().to_string()
}

/// Desktop-managed shortcuts must not compete with X11 grabs or portal registrations.
pub fn is_managed() -> bool {
    let Ok(desktop) = Desktop::current() else {
        return false;
    };
    let Ok(parent) = settings(desktop.parent, None) else {
        return false;
    };
    if !parent
        .strv(desktop.list)
        .iter()
        .any(|e| e.as_str() == desktop.entry)
    {
        return false;
    }
    let Ok(path) = desktop.path(desktop.entry) else {
        return false;
    };
    let Ok(custom) = settings(desktop.custom, Some(&path)) else {
        return false;
    };
    let binding = custom.value("binding");
    let matches = binding.get::<String>().is_some_and(|s| s == "Print")
        || binding
            .get::<Vec<String>>()
            .is_some_and(|s| s.iter().any(|s| s == "Print"));
    let command = custom.string("command");
    matches
        && glib::shell_parse_argv(&command).is_ok_and(|args| {
            args.len() == 2
                && args[1] == "--capture"
                && (std::env::current_exe().is_ok_and(|exe| exe.as_os_str() == args[0])
                    || args[0] == "/usr/bin/snappy"
                    || glib::home_dir().join(".local/bin/snappy").as_os_str() == args[0])
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exercises actual GSettings types without touching a user's desktop preferences.
    #[test]
    #[ignore = "Requires an isolated config directory, memory GSettings backend and desktop schemas"]
    fn installation_preserves_and_restores_shortcuts() {
        assert_eq!(std::env::var("GSETTINGS_BACKEND").unwrap(), "memory");
        assert!(glib::user_config_dir().starts_with("/tmp"));
        let desktop = Desktop::current().unwrap();
        let parent = settings(desktop.parent, None).unwrap();
        let builtin_id = desktop.builtins[0];
        let builtin = settings(builtin_id, None).unwrap();
        let key = if desktop.array_binding {
            "screenshot"
        } else {
            "show-screenshot-ui"
        };
        let original = vec!["Print", "<Alt>Print"];
        builtin.set_value(key, &original.to_variant()).unwrap();
        let other_entry = if desktop.array_binding {
            "existing".into()
        } else {
            format!("{}existing/", desktop.prefix)
        };
        parent
            .set_value(desktop.list, &vec![other_entry.clone()].to_variant())
            .unwrap();
        let other = settings(desktop.custom, Some(&desktop.path(&other_entry).unwrap())).unwrap();
        let other_binding = if desktop.array_binding {
            vec!["Print", "<Shift>Print"].to_variant()
        } else {
            "Print".to_variant()
        };
        other.set_value("binding", &other_binding).unwrap();
        install().unwrap();
        assert!(is_managed());
        assert_eq!(
            builtin
                .strv(key)
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>(),
            ["<Alt>Print"]
        );
        assert!(!other.value("binding").print(false).contains("'Print'"));
        install().unwrap(); // Reinstall must retain the first backup.
        let later_entry = if desktop.array_binding {
            "later".into()
        } else {
            format!("{}later/", desktop.prefix)
        };
        let entries = vec![
            other_entry.clone(),
            desktop.entry.into(),
            later_entry.clone(),
        ];
        parent
            .set_value(desktop.list, &entries.to_variant())
            .unwrap();
        restore().unwrap();
        assert!(!is_managed());
        assert_eq!(builtin.value(key), original.to_variant());
        assert_eq!(other.value("binding"), other_binding);
        assert_eq!(
            parent.value(desktop.list),
            vec![other_entry, later_entry].to_variant()
        );
        assert!(!backup_path().exists());
    }
}
