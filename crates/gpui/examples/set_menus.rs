#![cfg_attr(target_family = "wasm", no_main)]

use gpui::{
    App, Context, Global, Menu, MenuItem, SharedString, SystemMenuType, Window, WindowOptions,
    actions, div, prelude::*,
};
use gpui_platform::application;

struct SetMenus;

impl Render for SetMenus {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .bg(gpui::white())
            .size_full()
            .justify_center()
            .items_center()
            .text_xl()
            .text_color(gpui::black())
            .child("设置菜单示例")
    }
}

fn run_example() {
    application().run(|cx: &mut App| {
        cx.set_global(AppState::new());

        // Bring the menu bar to the foreground (so you can see the menu bar)
        cx.activate(true);
        // Register the `quit` function so it can be referenced
        // by the `MenuItem::action` in the menu bar
        cx.on_action(quit);
        cx.on_action(toggle_check);
        // Add menu items
        set_app_menus(cx);
        cx.open_window(WindowOptions::default(), |_, cx| cx.new(|_| SetMenus {}))
            .unwrap();
    });
}

#[cfg(not(target_family = "wasm"))]
fn main() {
    run_example();
}

#[cfg(target_family = "wasm")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    gpui_platform::web_init();
    run_example();
}

#[derive(PartialEq)]
enum ViewMode {
    List,
    Grid,
}

impl ViewMode {
    fn toggle(&mut self) {
        *self = match self {
            ViewMode::List => ViewMode::Grid,
            ViewMode::Grid => ViewMode::List,
        }
    }
}

impl Into<SharedString> for ViewMode {
    fn into(self) -> SharedString {
        match self {
            ViewMode::List => "列表",
            ViewMode::Grid => "网格",
        }
        .into()
    }
}

struct AppState {
    view_mode: ViewMode,
}

impl AppState {
    fn new() -> Self {
        Self {
            view_mode: ViewMode::List,
        }
    }
}

impl Global for AppState {}

fn set_app_menus(cx: &mut App) {
    let app_state = cx.global::<AppState>();
    cx.set_menus([Menu::new("set_menus").items([
        MenuItem::os_submenu("服务", SystemMenuType::Services),
        MenuItem::separator(),
        MenuItem::action("禁用项", gpui::NoAction).disabled(true),
        MenuItem::submenu(Menu::new("禁用子菜单").disabled(true)),
        MenuItem::separator(),
        MenuItem::action("列表模式", ToggleCheck).checked(app_state.view_mode == ViewMode::List),
        MenuItem::submenu(
            Menu::new("模式").items([
                MenuItem::action(ViewMode::List, ToggleCheck)
                    .checked(app_state.view_mode == ViewMode::List),
                MenuItem::action(ViewMode::Grid, ToggleCheck)
                    .checked(app_state.view_mode == ViewMode::Grid),
            ]),
        ),
        MenuItem::separator(),
        MenuItem::action("退出", Quit),
    ])]);
}

// Associate actions using the `actions!` macro (or `Action` derive macro)
actions!(set_menus, [Quit, ToggleCheck]);

// Define the quit function that is registered with the App
fn quit(_: &Quit, cx: &mut App) {
    println!("正在正常退出应用程序...");
    cx.quit();
}

fn toggle_check(_: &ToggleCheck, cx: &mut App) {
    let app_state = cx.global_mut::<AppState>();
    app_state.view_mode.toggle();
    set_app_menus(cx);
}
