// Drafts live in this account's private store; "new" creates one.
petal::route_file!(spec: petal::static_dir_spec(), list:
    vec![
        petal::writable("new"),
        petal::file("history.json"),
        petal::dir("drafts"),
    ]
);
