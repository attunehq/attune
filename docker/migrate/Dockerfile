FROM node:24-bookworm

WORKDIR /app
COPY ./package.json ./package-lock.json ./
RUN npm install
COPY ./prisma ./
