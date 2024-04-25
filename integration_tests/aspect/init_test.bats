load "common.bats"

setup() {
    cd "$TEST_REPO" || exit 1
}

@test 'aspect init should create functional workspace' {
    NEW_WKSP="$(mktemp -d)"
    (
        cd "$NEW_WKSP"
        run aspect init "."

        aspect run format
        aspect build //...
    )
}
