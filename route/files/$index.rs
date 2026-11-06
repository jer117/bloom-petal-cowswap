petal::route_file!(spec: petal::static_dir_spec(), list: {
    let mut children = vec![petal::file("status.json")];
    children.extend(petal::dir_names(&[
        "mainnet",
        "xdai",
        "arbitrum_one",
        "base",
        "sepolia",
    ]));
    children
});
