FROM rust AS builder

RUN apt-get update && \
    DEBIAN_FRONTEND=noninteractive apt-get install -y \
    protobuf-compiler \
    clang libclang-dev

ARG ETH_RPC_VERSION

WORKDIR /polkadot
COPY . /polkadot
RUN rustup component add rust-src

# If ETH_RPC_VERSION is set, install that version from crates.io.
# Otherwise build from the local source copy above.
RUN if [ -n "$ETH_RPC_VERSION" ]; then \
      cargo install --locked \
        pallet-revive-eth-rpc@${ETH_RPC_VERSION} \
        --bin eth-rpc \
        --root /out; \
    else \
      cargo build --locked --profile production \
        -p pallet-revive-eth-rpc --bin eth-rpc && \
      mkdir -p /out/bin && \
      cp target/production/eth-rpc /out/bin/eth-rpc; \
    fi

FROM docker.io/parity/base-bin:latest
COPY --from=builder /out/bin/eth-rpc /usr/local/bin

USER root
RUN useradd -m -u 1001 -U -s /bin/sh -d /polkadot polkadot && \
    # unclutter and minimize the attack surface
    rm -rf /usr/bin /usr/sbin && \
    # check if executable works in this container
    /usr/local/bin/eth-rpc --help

USER polkadot

# 8545 is the default port for the RPC server
# 9616 is the default port for the prometheus metrics
EXPOSE 8545 9616
ENTRYPOINT ["/usr/local/bin/eth-rpc"]

# We call the help by default
CMD ["--help"]
