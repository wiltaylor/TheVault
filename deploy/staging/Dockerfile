FROM python:3.13-slim-bookworm@sha256:a5a7017efc5969e00ae2b91c8eb0631049a569652aa4ffc4501769049ae45c72
RUN groupadd -g 1000 staging && useradd -u 1000 -g 1000 staging \
    && mkdir /data /config && chown 1000:1000 /data /config
COPY bin/ /usr/local/bin/
COPY staging.py /app/staging.py
USER 1000:1000
ENTRYPOINT ["python3", "-u", "/app/staging.py", "serve"]
