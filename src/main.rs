use bevy::prelude::*;
use space_game::game::GamePlugin;

fn main() {
    #[cfg(target_arch = "wasm32")]
    std::panic::set_hook(Box::new(|info| {
        if let Some(document) = web_sys::window().and_then(|w| w.document())
            && let Some(error) = document.get_element_by_id("error")
        {
            error.set_text_content(Some(&format!("The game could not continue: {info}")));
            let _ = error.set_attribute("data-panic", &info.to_string());
            let _ = error.set_attribute("style", "display: grid");
        }
    }));
    let window = Window {
        title: "Successor Stars — Battle Prototype".into(),
        resolution: (1280, 820).into(),
        #[cfg(target_arch = "wasm32")]
        canvas: Some("#game".into()),
        #[cfg(target_arch = "wasm32")]
        fit_canvas_to_parent: true,
        ..default()
    };
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin { primary_window: Some(window), ..default() }))
        .add_plugins(GamePlugin)
        .add_systems(Update, hide_splash)
        .run();
}

/// Removes the HTML loading screen once a few frames have rendered.
fn hide_splash(mut frames: Local<u32>) {
    *frames += 1;
    if *frames != 3 {
        return;
    }
    #[cfg(target_arch = "wasm32")]
    if let Some(el) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.get_element_by_id("loading")) {
        let _ = el.set_attribute("style", "display: none");
    }
}
