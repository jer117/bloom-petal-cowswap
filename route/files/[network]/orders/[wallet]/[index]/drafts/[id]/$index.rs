petal::route_file!(spec: petal::static_dir_spec(), list:
    vec![
        petal::file("order.json"),
        petal::writable("confirm"),
        petal::file("status.json"),
    ]
);
