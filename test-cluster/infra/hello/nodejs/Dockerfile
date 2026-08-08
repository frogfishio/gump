FROM node:20-bookworm-slim

WORKDIR /app

COPY hello/nodejs/package.json /app/package.json
COPY hello/nodejs/src /app/src

EXPOSE 4300

CMD ["node", "src/server.mjs"]