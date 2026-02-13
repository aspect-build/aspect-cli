pub const FILES: &[(&str, &str)] = &[
    ("MODULE.aspect", include_str!("../MODULE.aspect")),
    // config/
    (
        "config/builtins.axl",
        include_str!("../config/builtins.axl"),
    ),
    (
        "config/delivery.axl",
        include_str!("../config/delivery.axl"),
    ),
    ("config/lint.axl", include_str!("../config/lint.axl")),
    ("config/nolint.axl", include_str!("../config/nolint.axl")),
    // tasks/
    ("tasks/delivery.axl", include_str!("../tasks/delivery.axl")),
    ("tasks/migrate.axl", include_str!("../tasks/migrate.axl")),
    (
        "tasks/dummy_lint.axl",
        include_str!("../tasks/dummy_lint.axl"),
    ),
    (
        "tasks/dummy_format.axl",
        include_str!("../tasks/dummy_format.axl"),
    ),
    // lib/
    ("lib/deliveryd.axl", include_str!("../lib/deliveryd.axl")),
    ("lib/github.axl", include_str!("../lib/github.axl")),
    ("lib/linting.axl", include_str!("../lib/linting.axl")),
    ("lib/platform.axl", include_str!("../lib/platform.axl")),
    ("lib/sarif.axl", include_str!("../lib/sarif.axl")),
];
