fn sm_console_command(game: &mut crate::game::Game<'_>, line: &str, handled: &mut bool) {
    crate::game::squaremap_glue::perform_console_command(game, line, handled);
}
