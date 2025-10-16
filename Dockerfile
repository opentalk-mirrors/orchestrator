# SPDX-FileCopyrightText: OpenTalk GmbH <mail@opentalk.eu>
#
# SPDX-License-Identifier: EUPL-1.2

FROM rust:alpine AS builder

RUN mkdir /orchestrator
WORKDIR /orchestrator

ADD . /orchestrator

RUN apk --no-cache add musl-dev curl

RUN cargo build --release --locked -p orchestrator

FROM alpine:latest

RUN apk --no-cache update && apk --no-cache upgrade
RUN apk --no-cache add ca-certificates

ENV USERID=1000
ENV GROUPID=1000

RUN mkdir /orchestrator
WORKDIR /orchestrator

COPY --from=builder /orchestrator/target/release/orchestrator .

USER $USERID:$GROUPID

EXPOSE 11222
ENTRYPOINT [ "./orchestrator" ]
