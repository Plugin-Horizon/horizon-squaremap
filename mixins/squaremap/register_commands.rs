fn sm_register_commands(list: &mut Vec<crate::game::native_server::NativeCommand>) {
    crate::game::squaremap_glue::register_commands(list);
}
