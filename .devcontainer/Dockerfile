ARG VARIANT="1"
FROM mcr.microsoft.com/devcontainers/rust:${VARIANT}

# Build tools needed by some crates
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
       pkg-config \
       libssl-dev \
       mold \
    && rm -rf /var/lib/apt/lists/*

# bun — for building the SvelteKit console
RUN curl -fsSL https://bun.sh/install | bash
ENV PATH="/root/.bun/bin:${PATH}"

# just — task runner
RUN cargo install just --locked
