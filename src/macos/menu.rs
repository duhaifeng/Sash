//! The status-item menu. Like Magnet's `StatusMenu`, it is rebuilt each time it
//! opens, so it always reflects the active window's display and application.

use objc2::MainThreadMarker;
use objc2::sel;
use objc2_app_kit::{
    NSApplication, NSControlStateValueOn, NSEventModifierFlags, NSMenu, NSMenuItem,
};
use objc2_foundation::NSString;

use super::{AppDelegate, accessibility_trusted, with_daemon};
use crate::model::Command;
use crate::shortcut::Shortcut;

pub(super) fn rebuild(menu: &NSMenu, mtm: MainThreadMarker, target: &AppDelegate) {
    menu.removeAllItems();
    let Some((context, commands)) = with_daemon(|daemon| {
        let context = daemon.c.menu_context();
        let commands: Vec<Command> = daemon.c.commands.for_axis(context.axis).cloned().collect();
        (context, commands)
    }) else {
        return;
    };

    if !accessibility_trusted() {
        add(menu, "Not Authorized to Control Your Windows", None, target);
        add(menu, "Authorize…", Some(sel!(authorize:)), target);
        menu.addItem(&NSMenuItem::separatorItem(mtm));
    }

    for command in commands.iter().filter(|c| c.in_menu) {
        let item = add(menu, &command.name, Some(sel!(runCommand:)), target);
        if let Some((key, mask)) = command.shortcut.as_ref().and_then(key_equivalent) {
            item.setKeyEquivalent(&NSString::from_str(&key));
            item.setKeyEquivalentModifierMask(mask);
        }
        if command.separator_after {
            menu.addItem(&NSMenuItem::separatorItem(mtm));
        }
    }

    if let Some(app) = context.app {
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        if app.ignored_by_preferences {
            add(
                menu,
                &format!("“{}” Is Ignored by Preferences", app.name),
                None,
                target,
            );
        } else {
            let item = add(
                menu,
                &format!("Ignore “{}”", app.name),
                Some(sel!(toggleIgnore:)),
                target,
            );
            if app.ignored_by_menu {
                item.setState(NSControlStateValueOn);
            }
        }
    }

    menu.addItem(&NSMenuItem::separatorItem(mtm));
    add(menu, "Settings…", Some(sel!(openSettings:)), target);
    add(menu, "About Sash", Some(sel!(showAbout:)), target);
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    let quit = add(menu, "Quit Sash", Some(sel!(terminate:)), target);
    // `terminate:` belongs to NSApplication; the delegate does not implement it,
    // so an item targeting the delegate would be disabled.
    unsafe { quit.setTarget(Some(&NSApplication::sharedApplication(mtm))) };
}

fn add(
    menu: &NSMenu,
    title: &str,
    action: Option<objc2::runtime::Sel>,
    target: &AppDelegate,
) -> objc2::rc::Retained<NSMenuItem> {
    let item = unsafe {
        menu.addItemWithTitle_action_keyEquivalent(
            &NSString::from_str(title),
            action,
            &NSString::from_str(""),
        )
    };
    if action.is_some() {
        unsafe { item.setTarget(Some(target)) };
    }
    item
}

/// The key and modifiers a menu item shows for `shortcut`. Carbon performs the
/// real action; the item only displays the combination.
fn key_equivalent(shortcut: &Shortcut) -> Option<(String, NSEventModifierFlags)> {
    let key = if shortcut.key.len() == 1 {
        shortcut.key.to_ascii_lowercase()
    } else {
        let code = match shortcut.keysym {
            0xff52 => 0xF700, // Up
            0xff54 => 0xF701, // Down
            0xff51 => 0xF702, // Left
            0xff53 => 0xF703, // Right
            0xff50 => 0xF729, // Home
            0xff57 => 0xF72B, // End
            0xff55 => 0xF72C, // Page up
            0xff56 => 0xF72D, // Page down
            0xffff => 0xF728, // Forward delete
            0xff08 => 0x08,   // Backspace
            0xff0d => 0x0D,   // Return
            0xff09 => 0x09,   // Tab
            0xff1b => 0x1B,   // Escape
            0x20 => 0x20,     // Space
            _ => return None,
        };
        char::from_u32(code)?.to_string()
    };
    let m = shortcut.modifiers;
    let mut mask = NSEventModifierFlags::empty();
    for (on, flag) in [
        (m.ctrl, NSEventModifierFlags::Control),
        (m.alt, NSEventModifierFlags::Option),
        (m.shift, NSEventModifierFlags::Shift),
        (m.super_, NSEventModifierFlags::Command),
    ] {
        if on {
            mask |= flag;
        }
    }
    Some((key, mask))
}

/// The application menu bar Magnet's `MainMenu.nib` defines: About, Settings…,
/// Quit, File ▸ Close and the standard Edit items. Accessory apps do not show
/// it, but its key equivalents (⌘, ⌘W ⌘Q ⌘C ⌘V …) work while a window is key.
pub(super) fn install_main_menu(mtm: MainThreadMarker, target: &AppDelegate) {
    let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
    let bar = NSMenu::new(mtm);
    let submenu = |title: &str, items: &[(&str, Option<objc2::runtime::Sel>, &str, bool)]| {
        let menu = NSMenu::initWithTitle(mtm.alloc(), &NSString::from_str(title));
        for &(name, action, key, own_target) in items {
            if name.is_empty() {
                menu.addItem(&NSMenuItem::separatorItem(mtm));
                continue;
            }
            let item = unsafe {
                menu.addItemWithTitle_action_keyEquivalent(
                    &NSString::from_str(name),
                    action,
                    &NSString::from_str(key),
                )
            };
            if own_target {
                unsafe { item.setTarget(Some(target)) };
            }
        }
        let holder = NSMenuItem::new(mtm);
        holder.setSubmenu(Some(&menu));
        bar.addItem(&holder);
    };
    submenu(
        "Sash",
        &[
            ("About Sash", Some(sel!(showAbout:)), "", true),
            ("", None, "", false),
            ("Settings…", Some(sel!(openSettings:)), ",", true),
            ("", None, "", false),
            ("Quit Sash", Some(sel!(terminate:)), "q", false),
        ],
    );
    submenu("File", &[("Close", Some(sel!(performClose:)), "w", false)]);
    submenu(
        "Edit",
        &[
            ("Undo", Some(sel!(undo:)), "z", false),
            ("Redo", Some(sel!(redo:)), "Z", false),
            ("", None, "", false),
            ("Cut", Some(sel!(cut:)), "x", false),
            ("Copy", Some(sel!(copy:)), "c", false),
            ("Paste", Some(sel!(paste:)), "v", false),
            ("Select All", Some(sel!(selectAll:)), "a", false),
        ],
    );
    app.setMainMenu(Some(&bar));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_letters_and_arrows() {
        let (key, _) = key_equivalent(&"Ctrl+Alt+U".parse().unwrap()).unwrap();
        assert_eq!(key, "u");
        let (key, mask) = key_equivalent(&"Ctrl+Alt+Super+Left".parse().unwrap()).unwrap();
        assert_eq!(key, "\u{F702}");
        assert!(mask.contains(NSEventModifierFlags::Command | NSEventModifierFlags::Control));
    }
}
