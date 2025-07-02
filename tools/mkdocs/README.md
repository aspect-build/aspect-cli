# mkdocs

mkdocs is a quick way to serve the .md files under /docs for local preview

## Install on MacOS

mkdocs is a Python program. It can be install MacOS using pipx, which manages the virtual env for global Python programs.

```
brew install pipx
pipx install mkdocs
pipx inject mkdocs pymdown-extensions
pipx inject mkdocs mkdocs-shadcn
pipx ensurepath
```

## Serving

Once `mkdocs` is on your path, run the following:

```
cd tools/mkdocs
mkdocs serve
```

The docs are typically served at http://127.0.0.1:8000/.
