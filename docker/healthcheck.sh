#!/bin/bash
# The image's HEALTHCHECK: GET /api/health on the server's port answers 200. bash's /dev/tcp keeps
# curl out of the image.
set -eu
exec 3<>/dev/tcp/127.0.0.1/8080
printf 'GET /api/health HTTP/1.0\r\nHost: localhost\r\n\r\n' >&3
read -r -t 4 status <&3
case "$status" in
  "HTTP/"*" 200"*) exit 0 ;;
  *) exit 1 ;;
esac
