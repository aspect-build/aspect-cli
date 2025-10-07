---
title: Configuration Directives
sidebar_label: Directives
---

You can configure Aspect CLI using directives, which are specially-formatted
comments in `BUILD` files that govern behavior of the tool when visiting files
within the Bazel package rooted at that file.

There are generic [gazelle directives] that apply to any language as well as language specific directives:
* [js](https://github.com/aspect-build/orion/tree/main/language/js#directives)
* [go](https://github.com/bazelbuild/bazel-gazelle#directives)
* [proto](https://github.com/bazelbuild/bazel-gazelle#directives)
* [python](https://rules-python.readthedocs.io/en/latest/gazelle/docs/directives.html)
* [cc](https://github.com/EngFlow/gazelle_cc#custom-directives)

Aspect CLI provides additional generic directives from [Orion]:

<!-- prettier-ignore-start -->
| **Directive**                                           | **Default value**           |
| ------------------------------------------------------- | --------------------------- |
| `# gazelle:gitignore enabled\|disabled`                 | `disabled`                    |
<!-- prettier-ignore-end -->

[Orion]: https://github.com/aspect-build/orion
[gazelle directives]: https://github.com/bazelbuild/bazel-gazelle#directives
