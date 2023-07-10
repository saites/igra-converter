FROM opensuse/leap:15.4

RUN zypper ref
RUN zypper install -y -t pattern devel_basis

RUN curl https://sh.rustup.rs -sSf | \
    sh -s -- --default-toolchain stable -y

ENV PATH=/root/.cargo/bin:$PATH

RUN mkdir /project
WORKDIR /project
CMD ["cargo", "build", "--release"] 

