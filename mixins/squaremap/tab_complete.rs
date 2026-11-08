fn sm_tab_complete(command: &str, range: [usize; 2], suggestions: &mut Vec<horizon_client::mod_api::Suggestion>) {
    crate::game::squaremap_glue::tab_complete(command, range, suggestions);
}
