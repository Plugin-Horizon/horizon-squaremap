fn sm_perform_command(
    game: &mut crate::game::Game<'_>,
    sender: &crate::game::native_server::NativeServerPlayer,
    line: &str,
    handled: &mut bool,
) {
    crate::game::squaremap_glue::perform_command(game, sender, line, handled);
}
