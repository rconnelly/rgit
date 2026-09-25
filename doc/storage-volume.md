# Move the forge root onto a storage volume

Keep the path `/var/lib/rabun-git`. Attach a block volume (DigitalOcean Volumes, or any extra disk) and **mount it on that directory** so systemd, `RABUN_GIT_ROOT`, and `ReadWritePaths` stay unchanged. Only git data moves. `/etc/rabun-git` and `/usr/local/bin/rabun-git` stay on the droplet disk. If rgit-web is on the same host, `/opt/rgit-web` stays too.

Do this after [Ubuntu deploy](deploy-ubuntu.md). Git clients still use `ssh://git@HOST:2222/owner/name.git`.

## 1. Create and attach

In the provider UI, create a volume in the **same region** as the host and attach it. On DigitalOcean it appears as:

```text
/dev/disk/by-id/scsi-0DO_Volume_<name>
```

```bash
ls -l /dev/disk/by-id/scsi-0DO_Volume_*
```

Replace `scsi-0DO_Volume_rgit-data` below with that id.

## 2. Format once

This wipes the **volume**, not the droplet.

```bash
mkfs.ext4 -L rgit-data /dev/disk/by-id/scsi-0DO_Volume_rgit-data
```

Skip this if the volume already has a filesystem you intend to keep.

## 3. Stop writers, copy, swap the mount

```bash
systemctl stop rgit-web rabun-git   # omit rgit-web if that unit is not installed

mkdir -p /mnt/rgit-new
mount /dev/disk/by-id/scsi-0DO_Volume_rgit-data /mnt/rgit-new
rsync -aHAX --info=progress2 /var/lib/rabun-git/ /mnt/rgit-new/
umount /mnt/rgit-new

mv /var/lib/rabun-git /var/lib/rabun-git.rootdisk
mkdir /var/lib/rabun-git
mount /dev/disk/by-id/scsi-0DO_Volume_rgit-data /var/lib/rabun-git

chown rabun-git:rabun-git /var/lib/rabun-git
chmod 2770 /var/lib/rabun-git
chmod 0660 /var/lib/rabun-git/*.yaml 2>/dev/null || true
```

Trailing slashes on `rsync` matter. `mv` then mount so the old tree is not hidden under an overlay.

## 4. fstab

Do **not** use `nofail`. A missing volume would let the units start and write an empty forge on the root disk.

```bash
echo '/dev/disk/by-id/scsi-0DO_Volume_rgit-data /var/lib/rabun-git ext4 defaults,discard,noatime 0 2' >> /etc/fstab
findmnt /var/lib/rabun-git
systemctl daemon-reload
systemctl start rabun-git rgit-web
```

Optional: add `RequiresMountsFor=/var/lib/rabun-git` to `rabun-git.service` (and `rgit-web.service` if present) so they refuse to start if the volume is not mounted. Re-copy those units on the next pack/push unless you keep a drop-in:

```bash
mkdir -p /etc/systemd/system/rabun-git.service.d
printf '%s\n' '[Unit]' 'RequiresMountsFor=/var/lib/rabun-git' > /etc/systemd/system/rabun-git.service.d/volume.conf
systemctl daemon-reload
```

## 5. Check, then delete the copy

```bash
ls /var/lib/rabun-git/users.yaml /var/lib/rabun-git/repos
ssh -p 2222 -o IdentitiesOnly=yes git@HOST
```

When clone and (if used) the website look right:

```bash
rm -rf /var/lib/rabun-git.rootdisk
```

Do not change `RABUN_GIT_ROOT`. After a later `install.sh`, re-check the mount is still `2770` and group `rabun-git` so `rgit-web` can create users and repos.

Next: [Ubuntu deploy](deploy-ubuntu.md), [architecture](architecture.md).
