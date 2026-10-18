FROM archlinux:latest

RUN pacman -Sy --noconfirm sudo pacman-contrib shadow git && pacman -Scc --noconfirm
RUN useradd --create-home --shell /bin/bash mole && \
    printf 'mole ALL=(ALL) NOPASSWD: ALL\n' > /etc/sudoers.d/mole && \
    chmod 0440 /etc/sudoers.d/mole
COPY manifest.tsv /usr/local/share/mole-honeypot/manifest.tsv
COPY docker/seed-home.sh /usr/local/lib/mole-honeypot-seed
RUN bash /usr/local/lib/mole-honeypot-seed /home/mole && \
    chown -R mole:mole /home/mole && \
    mkdir -p /var/cache/pacman/pkg && \
    for package in mole-fixture-a mole-fixture-b; do \
      if [ "$package" = mole-fixture-a ]; then versions="1.0 1.1 1.2 1.3"; else versions="2.0 2.1 2.2 2.3"; fi; \
      for version in $versions; do \
        dd if=/dev/zero of="/var/cache/pacman/pkg/${package}-${version}-1-x86_64.pkg.tar.zst" bs=1024 count=1024 status=none; \
      done; \
    done
USER mole
ENV HOME=/home/mole
WORKDIR /home/mole
