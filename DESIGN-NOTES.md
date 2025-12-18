# How AerynOS delivers software to OS installs

## Software package metadata: manifest.*.bin

The `manifest.${ARCH}.bin` files contain all the metadata needed by the AerynOS tooling. For context, the `manifest.${ARCH}.jsonc` files are only there for git diff purposes and human-readable insight. They are completely ignored by the tooling.

    **Ikey Doherty**
    > our manifest.*.bin format is just a .stone in disguise
    > containing only a metadata payload with special fields
    > and the stone archive type flag is set to buildmanifest
    > sneaksy
    > (in fact, our repo format is also just a set of meta payloads in a stone file..)
    > but its also strongly typed, fixed headers, version agnostic header unpack and compressed with zstd with CRC checks 
    > soo. a little less weak than sounding
    > crc is actually xxh64 iirc

## Software distribution via *.stone packages

AerynOS distributes software via its custom `stone` format. This format was explicitly built to enable fast, deduplicated transmission and installation of software artefacts on target OS installs.

    > **Ikey Doherty**
    > Context: we dont mix layout + metadata (unlike in alpine, where tar records are used for metadata)
    > in fact we explicitly separate them
    > so a "normal" stone file has a meta payload with strongly typed/tagged key value pairs/sets
    > a content payload which is every unique file concatenated into a "megablob" and compressed singly 
    > an index payload which is a jump table into offsets in the unpacked content payload
    > to allow the xxhash128 keying
    > ie "position one is hash xyz"
    > and lastly there is the layout payload which is a meta-ish payload containing a set of records that define how the package is laid out on disk when installed
    > so the paths, file types, modes, link targets, permissions
    > and optionally for regular files, the xxh128 hash
    > so when we "cache" / install a package, in reality we're ripping the content payload out, then using the index payload to shard it into the unique assets in the store to build up the content addressable storage
    > we then merge the entries from metapayload + layoutpayload into the DBs
    > and we use the unique package "id" to key it, ie the hash for the `.stone`
    > internally moss has a notion of "State" whereby it maps your explicitly / transitive selections for a transaction into those pkgids
    > and during composition ("blit") we load all the required ids/etc/ and produce an in memory VFS structure using those "LayoutEntry"
    > and then blit in optimal order into a tree using linkat, mkdirat, etc.
    > we also do some graph ordering and reparenting to detect filesystem conflicts ahead of time
    > and to solve symlinked directories chicken/egg ordering
    > and detect those conflicts too..
    > anyway, the main sauce is then linkat for all the $hash -> $root/$path
    > and is delta-friendly (in future) by not locking paths to contents
    > In summary: "A lot more than a single tar file can do."
    > the vfs crate is actually borderline devil magic
    > https://github.com/AerynOS/os-tools/blob/main/crates/vfs/src/tree/mod.rs
    > https://github.com/AerynOS/os-tools/blob/main/crates/vfs/src/tree/builder.rs
    > but it does mean we can bake the view of the applied/installed OS ahead of time in memory and organise/optimise it
    > and we use that to make "new" installs each time under /.moss/root/staging
    > we then do some magic shit with linux containers (kernel namespaces) to enter the new system and run some triggers in an ephemeral copy
    > and eventually we swap that staging system with /usr using renameat2 / atomic exchange / rename
    > and ofc swap the one now at staging into an archived ID
    > as well as handling the boot management via blsforme..
    > i mean its basically systemd of package managers.
    > on crack
    > so when folks say "hey apk is super fast" im like "is it any fucking wonder"
    > it does nothing
    > look at us
    > only thing moss isnt doing is delivering pizzas on the side
