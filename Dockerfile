# Image de production de Synalyse : un binaire, aucune autre dépendance que PostgreSQL.
FROM rust:1-slim-trixie AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations
COPY tracker ./tracker
COPY web ./web
COPY vendor ./vendor
RUN cargo build --release --locked

FROM debian:trixie-slim
RUN useradd --system --uid 10001 --no-create-home synalyse
COPY --from=build /src/target/release/synalyse /usr/local/bin/synalyse
USER synalyse
ENV SYNALYSE_LISTEN=0.0.0.0:8292
EXPOSE 8292
ENTRYPOINT ["synalyse"]
CMD ["serve"]
