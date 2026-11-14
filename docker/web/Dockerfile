# The website (web/): public pages, My account and the admin panel. Built
# with the repository root as the context (see .dockerignore, which lets in
# only what the build needs); compose.yaml's web service runs it and the
# repository's "Website image" workflow publishes it.

FROM rust:1-alpine AS build
RUN apk add --no-cache musl-dev gcc
WORKDIR /src
COPY core core
COPY web web
# The pages embed the desktop app's shared stylesheet, fonts and frame images.
COPY desktop/ui/tokens.css desktop/ui/tokens.css
COPY desktop/ui/fonts desktop/ui/fonts
COPY desktop/ui/modern desktop/ui/modern
RUN cargo build --release --locked --manifest-path web/Cargo.toml

FROM alpine:3
RUN apk add --no-cache su-exec
COPY --from=build /src/web/target/release/web /web
# compose.yaml mounts nothing over these: the entrypoint is part of the image.
COPY docker/web/entrypoint.sh /entrypoint.sh
ENV DATA_DIR=/data
EXPOSE 8080
ENTRYPOINT ["/entrypoint.sh"]
CMD ["serve"]
