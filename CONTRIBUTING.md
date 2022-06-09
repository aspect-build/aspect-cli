Want to contribute? Great!

## Formatting/linting

We suggest using a pre-commit hook to automate this. First
[install pre-commit](https://pre-commit.com/#installation), then run

```shell
pre-commit install
pre-commit install --hook-type commit-msg
```

Otherwise the CI will yell at you about formatting/linting violations.

This also uses shellcheck so you may need to install if you are changing shell scripts
https://github.com/koalaman/shellcheck#user-content-installing
