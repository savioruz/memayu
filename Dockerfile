FROM rust:alpine AS builder

RUN apk add --no-cache musl-dev pkgconfig g++

WORKDIR /app
COPY . .

RUN cargo build --release --bin memayu --all-features && \
    strip target/release/memayu

FROM alpine:3.22

RUN apk add --no-cache ca-certificates && \
    adduser -D -h /data memayu

COPY --from=builder /app/target/release/memayu /usr/local/bin/memayu

ENV MEMAYU_LIBSQL_PATH=/data/memayu.db
ENV MEMAYU_PORT=8080

USER memayu
WORKDIR /data
EXPOSE 8080

ENTRYPOINT ["memayu"]
CMD ["serve"]
