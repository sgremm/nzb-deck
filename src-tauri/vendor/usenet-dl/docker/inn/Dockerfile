# Lightweight INN (InterNetNews) server for testing usenet-dl
#
# This provides a local NNTP server for deterministic end-to-end testing.
# Articles posted to this server can be downloaded via usenet-dl.

FROM debian:bookworm-slim

# Install INN2 and utilities
RUN apt-get update && apt-get install -y \
    inn2 \
    netcat-traditional \
    procps \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

# Create required directories
RUN mkdir -p /var/spool/news/articles \
    /var/spool/news/overview \
    /var/spool/news/incoming \
    /var/log/news \
    /run/news

# Copy configuration files
COPY newsgroups /etc/news/active
COPY inn.conf /etc/news/inn.conf
COPY readers.conf /etc/news/readers.conf

# Set proper permissions
RUN chown -R news:news /var/spool/news /var/log/news /run/news \
    && chmod 755 /var/spool/news /var/log/news /run/news

# Expose NNTP port
EXPOSE 119

# Start INN in foreground mode
USER news
CMD ["innd", "-d", "-f"]
