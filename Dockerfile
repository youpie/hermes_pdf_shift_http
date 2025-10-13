FROM rust:1.90

RUN apt update
RUN apt-get install libqpdf-dev clang -y && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/hermes_pdf_shift_http
COPY ./src ./src
COPY Cargo.lock ./
COPY Cargo.toml ./


RUN cargo install --path .

CMD ["hermes_pdf_shift_http"]