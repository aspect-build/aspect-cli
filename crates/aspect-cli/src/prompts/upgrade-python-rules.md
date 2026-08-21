# Upgrade this Python Bazel repository to Aspect rules_py

Add `aspect_rules_py` beside the existing setup and migrate one representative target before removing anything. Replace `@rules_python//python:defs.bzl` targets with the equivalent `@aspect_rules_py//py:defs.bzl` rules, `pip.parse` or `pip_install` with the bundled `uv` hub, and `@pip//package` labels with the matching underscore-normalised `@pypi//package` labels.

Move interpreter provisioning only after the migrated target selects and runs with the intended toolchain. Remove `rules_python`, requirements plumbing, and compatibility shims only after representative tests and the relevant CI platforms pass. Report packaging-visible metadata changes required by the new lock resolution.
