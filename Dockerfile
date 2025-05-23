FROM node:22-alpine

WORKDIR /usr/src/app

COPY package*.json ./
RUN npm install --production

COPY . .

EXPOSE 13470

ENV NODE_ENV=production

CMD ["node", "index.js"]
