# ffxiahbot, the auction house bot, at a pinned commit. compose.yaml's
# ahbot service runs it; the repository's "Auction house bot image"
# workflow publishes it so players don't build it themselves.
FROM python:3.13-slim

ARG AHBOT_REPO=https://github.com/AdamGagorik/ffxiahbot
ARG AHBOT_COMMIT=8a42d681e6c94437bade0d4d03d9a8ac9234db0f

ADD ${AHBOT_REPO}/archive/${AHBOT_COMMIT}.tar.gz /tmp/ahbot.tar.gz
# The price lists in bin/ aren't part of the Python package; keep them
# beside it.
RUN mkdir /tmp/src \
 && tar -xzf /tmp/ahbot.tar.gz --strip-components=1 -C /tmp/src \
 && pip install --no-cache-dir /tmp/src \
 && mkdir -p /opt/ahbot \
 && cp /tmp/src/bin/items.csv /tmp/src/bin/items75.csv /opt/ahbot/ \
 && rm -rf /tmp/src /tmp/ahbot.tar.gz

ENV PYTHONUNBUFFERED=1
# compose.yaml mounts this folder at /wrapper, so the script can change
# without a new image.
ENTRYPOINT ["/bin/sh", "/wrapper/run.sh"]
CMD ["broker"]
