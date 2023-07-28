FROM opensuse/leap:15.4

RUN zypper ref
RUN zypper install -y -t pattern devel_basis

ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH

RUN curl https://sh.rustup.rs -sSf | \
    sh -s -- \
    --no-modify-path \
    --profile minimal \
    --default-toolchain stable -y

RUN chmod -R a+w $RUSTUP_HOME $CARGO_HOME && \
    mkdir /usr/src/project
WORKDIR /usr/src/project
CMD ["cargo", "build", "--release"] 

