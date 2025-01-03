#!/bin/bash

installkernel() {
    return 0
}

check() {
    if [[ -x $systemdutildir/systemd ]] && [[ -x /usr/lib/moss/moss-fstx.sh ]]; then
       return 255
    fi

    return 1
}

depends() {
    return 0
}

install() {
    dracut_install /usr/lib/moss/moss-fstx.sh
    dracut_install /usr/bin/moss
    inst_simple "${systemdsystemunitdir}/moss-fstx.service"
    mkdir -p "${initdir}${systemdsystemconfdir}/initrd-root-fs.target.wants"
    ln_r "${systemdsystemunitdir}/moss-fstx.service" \
        "${systemdsystemconfdir}/initrd-root-fs.target.wants/moss-fstx.service"
}
