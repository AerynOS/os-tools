---
# SPDX-FileCopyrightText: 2023 AerynOS Developers
# SPDX-License-Identifier: MPL-2.0
---

# container

container is a crate to start a basic rootless container. Its goal is to create a safe and isolated environment where to build and/or test packages. Environment isolation is desired in that:

  * We don't want to pollute the *host* system with temporary artifacts
  * We want to work in a reproducible environment, for everyone (~~"it worked on my machine!"~~)
  * Certain applications may run arbitrary code while building. We want to be able do to that securely.

Emphasis is put on **rootless** container. Root permission must not be necessary, both for security and to reach a broader audience (including parent rootless containers, like Docker or Podman!).

## Containerization logic

Below is a walkthrough to build and enter a container. The logic is simple but dives into some technical details of Linux. Take your time to understand it. If you're feeling brave, you may try to reproduce the logic in a terminal using the [unshare](https://man7.org/linux/man-pages/man1/unshare.1.html) command.

  1. `clone` a child process. This is like a `fork` on steroids: We can pass it flags to avoid sharing certain resources with the parent, e.g. PIDs, network access, or even user/group IDs, which is the foundation of a rootless container. Let's call the child process "Bill".
  1. Bill has no idea who are the users and groups! It lives within a new user namespace. We must make it wait to build/test a package until we assigned some. To pause the execution, we use a pipe: the parent on one end, Bill on the other. A `read` blocks until there's data coming out of the pipe.
  1. Parent-side, we call `/usr/bin/newuidmap`, `/usr/bin/newgidmap` against Bill's PID to fix the point above. We can't do what those utilities do alone: setting UIDs and GIDs requires the `CAP_SETUID` capability which we don't have, unless we are root (remember: we want a root**less** container). `newuidmap`, `newuidmap` have that capability already set.
     - What IDs are we assigning to Bill? There's a Linux feature called "ID mapping": our user ID outside the container, say 100, can be 0 inside the container. What and how many sub-IDs are available are regulated by `/etc/sub{u,g}id` files. You guessed it right: we can pretend to be root inside a container! Everytime we write a file inside the container as root, will be actually written outside the container as user 100, that is us.
  1. With Bill's identity crisis over, the parent writes a sentinel value into the pipe that makes the `read` function unblock.
  1. Bills has an isolated user and group, but not an isolated filesystem. Since we are now privileged inside it, we're able to choose a directory as the root directory, and bind-mount important filesystems like tmpfs, procfs and sysfs. We also bind-mount devices like `/dev/zero` and `/dev/urandom`. Finally, we perform a `pivot_root` to hide the host filesystem for good.
     - `pivot_root` is more secure than the well-known `chroot`, in that it's not possible to escape it.
  1. Bill finally builds/tests the package as the parent waits for it to finish.

#### References

[`clone` syscall](https://linux.die.net/man/2/clone). [Linux namespaces](https://man7.org/linux/man-pages/man7/namespaces.7.html). [Linux user namespaces](https://man7.org/linux/man-pages/man7/user_namespaces.7.html) (a particular case of namespace). [`pipe`](https://linux.die.net/man/2/pipe). [newuidmap](https://man7.org/linux/man-pages/man1/newuidmap.1.html). [newgidmap](https://man7.org/linux/man-pages/man1/newgidmap.1.html). [`pivot_root`](https://linux.die.net/man/8/pivot_root).

## FAQ

#### Why don't you just use Docker, or Podman, or... ?

Those are full-fledged containerization solutions. They offer many features we don't need, and they have to be set up, or installed at best. Our logic is so simple we don't need to install anything except `newuidmap` and `newgidmap`, that is, it's built-in in our builder. We aim for an immediate packaging/testing experience.
