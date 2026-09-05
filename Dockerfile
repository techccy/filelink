# syntax=docker/dockerfile:1

FROM golang:1-alpine AS build
WORKDIR /src
ENV GOPROXY=https://goproxy.cn,direct
COPY go.mod go.sum ./
RUN go mod download
COPY . .
RUN CGO_ENABLED=0 go build -trimpath -ldflags="-s -w" -o /out/filelink .

FROM scratch
COPY --from=build /out/filelink /filelink
VOLUME /data
EXPOSE 8080
ENTRYPOINT ["/filelink"]
