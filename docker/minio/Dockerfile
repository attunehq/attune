FROM minio/minio:RELEASE.2025-04-08T15-41-24Z-cpuv1

ADD ./entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh
ENTRYPOINT [ "/bin/bash", "/entrypoint.sh" ]
