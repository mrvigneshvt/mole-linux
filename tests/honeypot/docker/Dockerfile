FROM archlinux:latest

RUN pacman -Sy --noconfirm sudo pacman-contrib shadow && pacman -Scc --noconfirm
RUN useradd --create-home --shell /bin/bash mole && \
    printf 'mole ALL=(ALL) NOPASSWD: ALL\n' > /etc/sudoers.d/mole && \
    chmod 0440 /etc/sudoers.d/mole
COPY docker/seed-home.sh /usr/local/lib/mole-honeypot-seed
COPY canary-manifest.txt /usr/local/share/mole-honeypot/canary-manifest.txt
RUN bash /usr/local/lib/mole-honeypot-seed /home/mole && \
    chown -R mole:mole /home/mole && \
    mkdir -p /var/cache/pacman/pkg && \
    touch /var/cache/pacman/pkg/fixture-1-1-x86_64.pkg.tar.zst \
          /var/cache/pacman/pkg/fixture-2-1-x86_64.pkg.tar.zst \
          /var/cache/pacman/pkg/fixture-3-1-x86_64.pkg.tar.zst
USER mole
WORKDIR /home/mole
