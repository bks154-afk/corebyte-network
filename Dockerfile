FROM rust:1.75-slim as builder
WORKDIR /app
COPY . .
RUN cargo build --release --bin block

FROM debian:bookworm-slim
WORKDIR /app
RUN apt-get update && apt-get install -y libssl-dev ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/block /app/block

EXPOSE 3333
EXPOSE 8080

CMD ["./block"]
