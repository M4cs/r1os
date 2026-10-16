# The website as Railway builds it (railway.json), from the repo's root: the Bun workspace installed from bun.lock (the
# site imports packages/icons and the shell's and companion's sources), the site built, and web/dist served by
# web/serve.ts on $PORT. A Dockerfile rather than Railpack's detection, which picks Rust from the root's Cargo.toml.
#
#     docker build -f web/Dockerfile -t r1os-web . && docker run --rm -e PORT=8080 -p 8080:8080 r1os-web

FROM oven/bun:1.3.2 AS build
WORKDIR /app
COPY . .
RUN bun install --frozen-lockfile
RUN bun run --filter @r1os/web build

FROM oven/bun:1.3.2
WORKDIR /app
COPY --from=build /app/web/dist web/dist
COPY web/serve.ts web/serve.ts
USER bun
ENV PORT=8080
EXPOSE 8080
CMD ["bun", "web/serve.ts"]
