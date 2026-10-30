//! The menu bar. Every item targets the app delegate; plain-letter shortcuts
//! (R, L, F, P …) are real key equivalents with an empty modifier mask, so
//! they show up in the menus and work from anywhere in the window.

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{sel, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSApplication, NSEventModifierFlags, NSMenu, NSMenuItem};
use objc2_foundation::NSString;

use crate::view::ns;

const CMD: NSEventModifierFlags = NSEventModifierFlags::Command;
const CMD_SHIFT: NSEventModifierFlags = NSEventModifierFlags::Command.union(NSEventModifierFlags::Shift);
const CMD_OPT: NSEventModifierFlags = NSEventModifierFlags::Command.union(NSEventModifierFlags::Option);
const NONE: NSEventModifierFlags = NSEventModifierFlags::empty();

// Function-key code points AppKit uses in key equivalents.
const LEFT: &str = "\u{F702}";
const RIGHT: &str = "\u{F703}";
const UP: &str = "\u{F700}";
const DOWN: &str = "\u{F701}";
const HOME: &str = "\u{F729}";
const END: &str = "\u{F72B}";
const F2: &str = "\u{F705}";
const FWD_DELETE: &str = "\u{F728}";
const BACKSPACE: &str = "\u{8}";

struct Item {
    title: &'static str,
    action: Option<Sel>,
    key: &'static str,
    mask: NSEventModifierFlags,
}

fn item(title: &'static str, action: Sel, key: &'static str, mask: NSEventModifierFlags) -> Item {
    Item { title, action: Some(action), key, mask }
}

fn sep() -> Item {
    Item { title: "-", action: None, key: "", mask: NONE }
}

fn build_menu(mtm: MainThreadMarker, title: &str, target: &AnyObject, items: &[Item]) -> Retained<NSMenu> {
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &ns(title));
    for it in items {
        if it.action.is_none() {
            menu.addItem(&NSMenuItem::separatorItem(mtm));
            continue;
        }
        let mi = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &ns(it.title),
                it.action,
                &NSString::from_str(it.key),
            )
        };
        mi.setKeyEquivalentModifierMask(it.mask);
        unsafe { mi.setTarget(Some(target)) };
        menu.addItem(&mi);
    }
    menu
}

pub fn install(mtm: MainThreadMarker, app: &NSApplication, target: &AnyObject) {
    let main = NSMenu::initWithTitle(NSMenu::alloc(mtm), &ns("MainMenu"));

    // -- Application menu ---------------------------------------------------
    let app_menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &ns("MacView"));
    let about = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &ns("About MacView"),
            Some(sel!(orderFrontStandardAboutPanel:)),
            &ns(""),
        )
    };
    app_menu.addItem(&about);
    app_menu.addItem(&NSMenuItem::separatorItem(mtm));
    let hide = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm), &ns("Hide MacView"), Some(sel!(hide:)), &ns("h"))
    };
    app_menu.addItem(&hide);
    let hide_others = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &ns("Hide Others"),
            Some(sel!(hideOtherApplications:)),
            &ns("h"),
        )
    };
    hide_others.setKeyEquivalentModifierMask(CMD_OPT);
    app_menu.addItem(&hide_others);
    app_menu.addItem(&NSMenuItem::separatorItem(mtm));
    let quit = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm), &ns("Quit MacView"), Some(sel!(terminate:)), &ns("q"))
    };
    app_menu.addItem(&quit);
    let app_item = NSMenuItem::new(mtm);
    app_item.setSubmenu(Some(&app_menu));
    main.addItem(&app_item);

    // -- File ----------------------------------------------------------------
    let file = build_menu(
        mtm,
        "File",
        target,
        &[
            item("Open…", sel!(openDocument:), "o", CMD),
            item("Reload", sel!(reloadImage:), "r", CMD_SHIFT),
            sep(),
            item("Save", sel!(saveDocument:), "s", CMD),
            item("Save As…", sel!(saveDocumentAs:), "s", CMD_SHIFT),
            sep(),
            item("Rename…", sel!(renameFile:), F2, NONE),
            item("Move to Trash", sel!(moveToTrash:), FWD_DELETE, NONE),
            sep(),
            item("Reveal in Finder", sel!(revealInFinder:), "r", CMD_OPT),
            item("Properties", sel!(showProperties:), "p", NONE),
            sep(),
            item("Close Window", sel!(performClose:), "w", CMD),
        ],
    );
    let file_item = NSMenuItem::new(mtm);
    file_item.setSubmenu(Some(&file));
    main.addItem(&file_item);

    // -- Edit ----------------------------------------------------------------
    let edit = build_menu(
        mtm,
        "Edit",
        target,
        &[
            item("Undo", sel!(undoImage:), "z", CMD),
            sep(),
            item("Copy Image", sel!(copyImage:), "c", CMD),
            item("Paste as New Image", sel!(pasteImage:), "v", CMD),
            sep(),
            item("Select All", sel!(selectAll:), "a", CMD),
            item("Clear Selection", sel!(clearSelection:), "\u{1b}", NONE),
            item("Crop to Selection", sel!(cropToSelection:), "y", CMD),
        ],
    );
    let edit_item = NSMenuItem::new(mtm);
    edit_item.setSubmenu(Some(&edit));
    main.addItem(&edit_item);

    // -- Image ---------------------------------------------------------------
    let image = build_menu(
        mtm,
        "Image",
        target,
        &[
            item("Resize/Resample…", sel!(resizeImage:), "r", CMD),
            item("Canvas Size…", sel!(canvasSize:), "r", CMD_OPT.union(NSEventModifierFlags::Shift)),
            sep(),
            item("Rotate Right", sel!(rotateRight:), "r", NONE),
            item("Rotate Left", sel!(rotateLeft:), "l", NONE),
            item("Rotate 180°", sel!(rotate180:), "l", CMD_SHIFT),
            item("Flip Horizontal", sel!(flipHorizontal:), "h", NONE),
            item("Flip Vertical", sel!(flipVertical:), "v", NONE),
            sep(),
            item("Convert to Grayscale", sel!(grayscale:), "g", CMD_SHIFT),
            item("Negative", sel!(negative:), "n", CMD_SHIFT),
            item("Auto Adjust Colors", sel!(autoAdjust:), "u", CMD_SHIFT),
        ],
    );
    let image_item = NSMenuItem::new(mtm);
    image_item.setSubmenu(Some(&image));
    main.addItem(&image_item);

    // -- View ----------------------------------------------------------------
    let view = build_menu(
        mtm,
        "View",
        target,
        &[
            item("Full Screen", sel!(macviewFullScreen:), "f", NONE),
            item("Fit to Window / 1:1", sel!(toggleFitActual:), "1", NONE),
            sep(),
            item("Zoom In", sel!(zoomIn:), "+", NONE),
            item("Zoom Out", sel!(zoomOut:), "-", NONE),
            item("Fit to Window", sel!(zoomFit:), "0", CMD),
            item("Actual Size", sel!(zoomActual:), "1", CMD),
            sep(),
            item("Transparency Checkerboard", sel!(toggleChecker:), "t", CMD_SHIFT),
        ],
    );
    let view_item = NSMenuItem::new(mtm);
    view_item.setSubmenu(Some(&view));
    main.addItem(&view_item);

    // -- Go ------------------------------------------------------------------
    let go = build_menu(
        mtm,
        "Go",
        target,
        &[
            item("Next Image", sel!(nextImage:), " ", NONE),
            item("Previous Image", sel!(prevImage:), BACKSPACE, NONE),
            item("Next (→)", sel!(nextImage:), RIGHT, NONE),
            item("Previous (←)", sel!(prevImage:), LEFT, NONE),
            item("Next (↓)", sel!(nextImage:), DOWN, NONE),
            item("Previous (↑)", sel!(prevImage:), UP, NONE),
            sep(),
            item("First Image", sel!(firstImage:), HOME, NONE),
            item("Last Image", sel!(lastImage:), END, NONE),
        ],
    );
    let go_item = NSMenuItem::new(mtm);
    go_item.setSubmenu(Some(&go));
    main.addItem(&go_item);

    // -- Window --------------------------------------------------------------
    let window = NSMenu::initWithTitle(NSMenu::alloc(mtm), &ns("Window"));
    let minimize = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &ns("Minimize"),
            Some(sel!(performMiniaturize:)),
            &ns("m"),
        )
    };
    window.addItem(&minimize);
    let window_item = NSMenuItem::new(mtm);
    window_item.setSubmenu(Some(&window));
    main.addItem(&window_item);
    app.setWindowsMenu(Some(&window));

    app.setMainMenu(Some(&main));
}
