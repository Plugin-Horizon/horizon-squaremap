fn sm_register_config(list: &mut Vec<crate::game::native_configs::NativeConfig>) {
    list.push(crate::game::native_configs::NativeConfig::new(
        "squaremap",
        crate::game::native_configs::NativeConfigKind::Server,
        horizon_squaremap::default_config(),
    ));
}
