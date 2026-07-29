pub const EXT4: &str = "24 1 8:1 / / rw,relatime - ext4 /dev/sda1 rw\n";
pub const NFS: &str = "31 24 0:44 / /mnt/edge rw - nfs4 server:/edge rw\n";
pub const SMB: &str = "32 24 0:45 / /mnt/smb rw - cifs //server/share rw\n";
pub const BIND: &str = "33 24 8:1 /var/backups /mnt/bind rw - ext4 /dev/sda1 rw\n";
