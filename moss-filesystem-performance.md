---
# SPDX-FileCopyrightText: 2025 AerynOS Developers
# SPDX-License-Identifier: MPL-2.0
---

# Moss vs. filesystem speed

- i7-6700K@4.6GHz
- 24GB DDR4-2400
- Toshiba SATA-600 SSD

## System details:

```
󰌽 ermo@blueshift:~
❯ sudo fdisk -l /dev/sdb
Disk /dev/sdb: 119,24 GiB, 128035676160 bytes, 250069680 sectors
Disk model: TOSHIBA THNSNH12
Units: sectors of 1 * 512 = 512 bytes
Sector size (logical/physical): 512 bytes / 512 bytes
I/O size (minimum/optimal): 512 bytes / 512 bytes
Disklabel type: gpt
Disk identifier: 450D5CFC-C712-43DF-A6F9-5A8B05D058F3

Device       Start       End   Sectors  Size Type
/dev/sdb1     2048    526335    524288  256M EFI System
/dev/sdb2   526336   8914943   8388608    4G Linux extended boot
/dev/sdb3  8914944 250068991 241154048  115G Linux root (x86-64)
󰌽 ermo@blueshift:~
❯ sudo lsblk -f /dev/sdb
NAME   FSTYPE FSVER LABEL UUID                                 FSAVAIL FSUSE% MOUNTPOINTS
sdb
├─sdb1 vfat   FAT16       2D82-2EC6
├─sdb2 vfat   FAT32       2F6C-E5A6
└─sdb3 f2fs   1.16                                               94,7G    18% /
󰌽 ermo@blueshift:~
❯ grep 'model name' /proc/cpuinfo |uniq
model name	: Intel(R) Core(TM) i7-6700K CPU @ 4.00GHz
```


### xfs hot cache

```
# balanced CPU governor
󰌽 ermo@blueshift:~
❯ sudo moss rm nano
The following package(s) will be removed:

nano  8.3-33

✔  Do you wish to continue?  · yes
Removed nano

231613 entries blitted in 6.06s (38.2k / s)
󰌽 ermo@blueshift:~
❯ sudo moss it -y nano
The following package(s) will be installed:

nano  8.3-33

Installed nano (cached)

231707 entries blitted in 6.13s (37.8k / s)
```


### xfs cold cache

```
# balanced CPU governor
󰌽 ermo@blueshift:~
❯ sudo sync && echo 3 | sudo tee /proc/sys/vm/drop_caches && sudo moss rm -y nano
3
The following package(s) will be removed:

nano  8.3-33

Removed nano

231613 entries blitted in 11.85s (19.5k / s)
󰌽 ermo@blueshift:~
❯ sudo sync && echo 3 | sudo tee /proc/sys/vm/drop_caches && sudo moss it -y nano
3
The following package(s) will be installed:

nano  8.3-33

Installed nano (cached)

231707 entries blitted in 12.01s (19.3k / s)
```


### f2fs hot cache

```
# Performance CPU governor
󰌽 ermo@blueshift:/run/media/ermo/5305e180-315c-4ecd-9b92-813de85eb68d/home/ermo/󰈙
❯ sudo moss rm nano
[sudo: authenticate] Password:
The following package(s) will be removed:

nano  8.3-33

✔  Do you wish to continue?  · yes
Removed nano

231613 entries blitted in 8.27s (28.0k / s)
󰌽 ermo@blueshift:/run/media/ermo/5305e180-315c-4ecd-9b92-813de85eb68d/home/ermo/󰈙
❯ sudo moss it nano
The following package(s) will be installed:

nano  8.3-33

✔  Do you wish to continue?  · yes
Installed nano (cached)

231707 entries blitted in 7.84s (29.5k / s)
```


### f2fs cold cache

```
# Performance CPU governor
󰌽 ermo@blueshift:~
❯ sudo sync && echo 3 | sudo tee /proc/sys/vm/drop_caches
3
󰌽 ermo@blueshift:~
❯ sudo moss rm nano
The following package(s) will be removed:

nano  8.3-33

✔  Do you wish to continue?  · yes
Removed nano

231613 entries blitted in 73.20s (3.2k / s)
󰌽 ermo@blueshift:~
❯ sudo sync && echo 3 | sudo tee /proc/sys/vm/drop_caches
3
󰌽 ermo@blueshift:~
❯ sudo moss it nano
The following package(s) will be installed:

nano  8.3-33

✔  Do you wish to continue?  · yes
Installed nano (cached)

231707 entries blitted in 73.98s (3.1k / s)
```

### ext4 hot cache

```
# balanced CPU governor
󰌽 ermo@blueshift:~
❯ sudo moss rm nano
The following package(s) will be removed:

nano  8.3-33

✔  Do you wish to continue?  · yes
Removed nano

231613 entries blitted in 5.58s (41.5k / s)
󰌽 ermo@blueshift:~
❯ sudo moss it nano
The following package(s) will be installed:

nano  8.3-33

✔  Do you wish to continue?  · yes
Installed nano (cached)

231707 entries blitted in 5.99s (38.7k / s)
```

### ext4 cold cache

```
# balanced CPU governor
󰌽 ermo@blueshift:~
❯ sudo sync && echo 3 |sudo tee /proc/sys/vm/drop_caches
3
󰌽 ermo@blueshift:~
❯ sudo moss rm nano
The following package(s) will be removed:

nano  8.3-33

✔  Do you wish to continue?  · yes
Removed nano

231613 entries blitted in 63.34s (3.7k / s)
󰌽 ermo@blueshift:~
❯ sudo sync && echo 3 |sudo tee /proc/sys/vm/drop_caches
3
󰌽ermo@blueshift:~
❯ sudo moss it nano
The following package(s) will be installed:

nano  8.3-33

✔  Do you wish to continue?  · yes
Installed nano (cached)

231707 entries blitted in 62.60s (3.7k / s)
```
