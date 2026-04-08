use baumhard::util::color::Color;
use crate::application::common::GameActivity;
use crate::application::game_concepts::{LocalGameHost, LocalGameState, Scene, World};

pub fn launch_main_menu(host: &mut LocalGameHost) {
    let mut main_menu_state = LocalGameState::new();
    let mut main_menu_world = World::new(
        "Main Menu".to_string(),
    );
    let mut main_menu_scene = Scene::new(
        "Root".to_string(),
        GameActivity::Menu,
        Color::black(),
    );

    main_menu_world.scenes.push(main_menu_scene);
    main_menu_state.worlds.push(main_menu_world);

    host.state = Some(main_menu_state);
}
