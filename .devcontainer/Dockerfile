# Start with the standard devcontainers base (Ubuntu-based)
FROM mcr.microsoft.com/devcontainers/base:ubuntu

# Install dependencies (direnv + curl for downloading bazelisk)
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
       direnv curl ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Install bazelisk (latest release)
RUN curl -L https://github.com/bazelbuild/bazelisk/releases/latest/download/bazelisk-linux-amd64 \
      -o /usr/local/bin/bazelisk \
    && chmod +x /usr/local/bin/bazelisk \
    && ln -s /usr/local/bin/bazelisk /usr/local/bin/bazel

# Hook direnv into bash (default Codespaces shell)
RUN echo 'eval "$(direnv hook bash)"' >> /etc/bash.bashrc