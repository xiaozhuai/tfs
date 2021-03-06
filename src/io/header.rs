//! Disk header parsing.
//!
//! The disk header provides information on how to read a TFS disk. This module parses and
//! interprets the disk header so it is meaningful to the programmer.

/// The size of the disk header.
///
/// This should be a multiple of the cluster size.
const DISK_HEADER_SIZE: usize = 4096;
/// The current version number.
///
/// The versioning scheme divides this number into two parts. The 16 most significant bits identify
/// breaking changes. For two version A to be able to read an image written by version B, two
/// requirements must hold true:
///
/// 1. A must be greater than or equal to B.
/// 2. A and B must have equal higher parts.
const VERSION_NUMBER: u32 = 0;
/// The magic number of images with partial TFS compatibility.
const PARTIAL_COMPATIBILITY_MAGIC_NUMBER: &[u8] = b"~TFS fmt";
/// The magic number of images with total TFS compatibility.
const TOTAL_COMPATIBILITY_MAGIC_NUMBER: &[u8] = b"TFS fmt ";

quick_error! {
    /// A disk header reading error.
    enum ParseError {
        /// Unknown format (not TFS).
        UnknownFormat {
            description("Unknown format (not TFS).")
        }
        /// The state flag is corrupt.
        CorruptStateFlag {
            description("Corrupt state flag.")
        }
        /// The cipher field is corrupt.
        CorruptCipher {
            description("Corrupt cipher option.")
        }
        /// The encryption parameters is corrupt.
        CorruptEncryptionParameters {
            description("Corrupt encryption paramters.")
        }
        /// The state block address is corrupt.
        CorruptStateBlockAddress {
            description("Corrupt state block address.")
        }
        /// The version number is corrupt.
        CorruptVersionNumber {
            description("Corrupt version number.")
        }
        /// The version is incompatible with this implementation.
        ///
        /// The version number is given by some integer. If the higher half of the integer does not
        /// match, the versions are incompatible and this error is returned.
        IncompatibleVersion {
            description("Incompatible version.")
        }
        /// Unknown/unsupported (implementation-specific) cipher option.
        UnknownCipher {
            description("Unknown cipher option.")
        }
        /// Invalid/nonexistent cipher option.
        ///
        /// Note that this is different from `UnknownCipher`, as it is necessarily invalid and not just
        /// implementation-specific.
        InvalidCipher {
            description("Invalid cipher option.")
        }
        /// Unknown state flag value.
        UnknownStateFlag {
            description("Unknown state flag.")
        }
    }
}

/// TFS magic number.
#[derive(PartialEq, Eq, Clone, Copy)]
enum MagicNumber {
    /// The image is partially compatible with the official TFS specification.
    PartialCompatibility,
    /// The image is completely compatible with the official TFS specification.
    TotalCompatibility,
}

impl TryFrom<&[u8]> for MagicNumber {
    type Err = ParseError;

    fn from(string: &[u8]) -> Result<MagicNumber, ParseError> {
        match string {
            // Partial compatibility.
            PARTIAL_COMPATIBILITY_MAGIC_NUMBER => Ok(MagicNumber::PartialCompatibility),
            // Total compatibility.
            TOTAL_COMPATIBILITY_MAGIC_NUMBER => Ok(MagicNumber::TotalCompatibility),
            // Unknown format; abort.
            _ => Err(ParseError::UnknownFormat),
        }
    }
}

impl Into<&'static [u8]> for MagicNumber {
    fn into(self) -> &[u8] {
        match self {
            MagicNumber::TotalCompatibility => TOTAL_COMPATIBILITY_MAGIC_NUMBER,
            MagicNumber::PartialCompatibility => PARTIAL_COMPATIBILITY_MAGIC_NUMBER,
        }
    }
}

/// Cipher option.
#[derive(PartialEq, Eq, Clone, Copy)]
enum Cipher {
    /// Disk encryption disabled.
    Identity = 0,
    /// Use the SPECK cipher.
    Speck128 = 1,
}

impl TryFrom<u16> for Cipher {
    type Err = ParseError;

    fn try_from(from: u16) -> Result<Cipher, ParseError> {
        match from {
            // Aye aye, encryption is disabled.
            0 => Ok(Cipher::Identity),
            // Wooh! Encryption on.
            1 => Ok(Cipher::Speck128),
            // These are implementation-specific ciphers which are unsupported in this (official)
            // implementation.
            1 << 15... => Err(ParseError::UnknownCipher),
            // This cipher is invalid by current revision.
            _ => Err(ParseError::InvalidCipher),
        }
    }
}

/// State flag.
///
/// The state flag defines the state of the disk, telling the user if it is in a consistent
/// state or not. It is important for doing non-trivial things like garbage-collection, where the
/// disk needs to enter an inconsistent state for a small period of time.
#[derive(PartialEq, Eq, Clone, Copy)]
enum StateFlag {
    /// The disk was properly closed and shut down.
    Closed = 0,
    /// The disk is active/was forcibly shut down.
    Open = 1,
    /// The disk is in an inconsistent state.
    ///
    /// Proceed with caution.
    Inconsistent = 2,
}

/// The disk header.
#[derive(Default, PartialEq, Eq, Clone, Copy)]
struct DiskHeader {
    /// The magic number.
    magic_number: MagicNumber,
    /// The version number.
    version_number: u32,
    /// The cipher.
    cipher: Cipher,
    /// The encryption paramters.
    ///
    /// These are used as defined by the choice of cipher. Some ciphers might use it for salt or
    /// settings, and others not use it at all.
    encryption_parameters: [u8; 16],
    /// The address of the state block.
    state_block_address: clusters::Pointer,
    /// The state flag.
    consistency_flag: StateFlag,
}

impl DiskHeader {
    /// Parse the disk header from some sequence of bytes.
    ///
    /// This will construct it into memory while performing error checks on the header to ensure
    /// correctness.
    fn decode(buf: &[u8]) -> Result<DiskHeader, ParseError> {
        // Start with some default value, which will be filled out later.
        let mut ret = DiskHeader::default();

        // # Introducer Section
        //
        // This section has the purpose of defining the implementation, version, and type of the
        // disk image. It is rarely changed unless updates or reformatting happens.

        // Load the magic number.
        ret.magic_number = MagicNumber::try_from(&buf[..8])?;

        // Load the version number.
        ret.version_number = LittleEndian::read(buf[8..]);
        // Right after the version number, the same number follows, but bitwise negated. Make sure
        // that these numbers match (if it is bitwise negated). The reason for using this form of
        // code rather than just repeating it as-is is that if one overwrites all bytes with a
        // constant value, like zero, it won't be detected.
        if ret.version_number == !LittleEndian::read(buf[12..]) {
            // Check if the version is compatible. If the higher half doesn't match, there were a
            // breaking change. Otherwise, if the version number is lower or equal to the current
            // version, it's compatible.
            if ret.version_number >> 16 != VERSION_NUMBER >> 16 || ret.version_number > VERSION_NUMBER {
                // The version is not compatible; abort.
                return Err(ParseError::IncompatibleVersion);
            }
        } else {
            // The version number is corrupt; abort.
            return Err(ParseError::CorruptVersionNumber);
        }

        // # Encryption section
        //
        // This section contains information about how the disk was encrypted, if at all.

        // Load the encryption algorithm choice.
        ret.cipher = Cipher::try_from(LittleEndian::read(buf[64..]))?;
        // Repeat the bitwise negation.
        if ret.cipher as u16 != !LittleEndian::read(buf[66..]) {
            // The cipher option is corrupt; abort.
            return Err(ParseError::CorruptCipher);
        }

        // Load the encryption parameters (e.g. salt).
        self.encryption_parameters.copy_from_slice(&buf[68..84]);
        // Repeat the bitwise negation.
        if self.encryption_parameters.iter().eq(buf[84..100].iter().map(|x| !x)) {
            // The encryption parameters are corrupt; abort.
            return Err(ParseError::CorruptEncryptionParameters);
        }

        // # State section
        //
        // This section holds the state of disk and pointers to information on the state of the
        // file system.

        // Load the state block pointer.
        ret.state_block_address = clusters::Pointer::new(LittleEndian::read(buf[128..]));
        // Repeat the bitwise negation.
        if ret.state_block_address as u64 != !LittleEndian::read(buf[136..]) {
            // The state block address is corrupt; abort.
            return Err(ParseError::CorruptStateBlockAddress);
        }

        // Load the state flag.
        self.consistency_flag = StateFlag::from(buf[144])?;
        // Repeat the bitwise negation.
        if self.consistency_flag as u8 != !buf[145] {
            // The state flag is corrupt; abort.
            return Err(ParseError::CorruptStateFlag);
        }
    }

    /// Encode the header into a sector-sized buffer.
    fn encode(&self) -> [u8; disk::SECTOR_SIZE] {
        // Create a buffer to hold the data.
        let mut buf = [0; disk::SECTOR_SIZE];

        // Write the magic number.
        buf[..8].copy_from_slice(self.magic_number.into());

        // Write the current version number.
        LittleEndian::write(&mut buf[8..], VERSION_NUMBER);
        LittleEndian::write(&mut buf[12..], !VERSION_NUMBER);

        // Write the cipher algorithm.
        LittleEndian::write(&mut buf[64..], self.cipher as u16);
        LittleEndian::write(&mut buf[66..], !self.cipher as u16);

        // Write the encryption parameters.
        buf[68..84].copy_from_slice(self.encryption_parameters);
        for (a, b) in buf[84..100].iter_mut().zip(self.encryption_parameters) {
            *a = !b;
        };

        // Write the state block address.
        LittleEndian::write(&mut buf[128..], self.state_block_address);
        LittleEndian::write(&mut buf[136..], !self.state_block_address);

        // Write the state flag.
        buf[144] = self.consistency_flag as u8;
        buf[145] = !self.consistency_flag as u8;

        buf
    }
}

/// A driver transforming a normal disk into a header-less decrypted disk.
///
/// This makes it more convinient to work with.
struct Driver<D: Disk> {
    /// The cached disk header.
    ///
    /// The disk header contains various very basic information about the disk and how to interact
    /// with it.
    ///
    /// In reality, we could fetch this from the `disk` field as-we-go, but that hurts performance,
    /// so we cache it in memory.
    pub header: header::DiskHeader,
    /// The inner disk.
    disk: D,
    /// The cipher and key.
    cipher: crypto::Cipher,
}

/// A driver loading error.
enum OpenError {
    /// The state flag was set to "inconsistent".
    InconsistentState {
        description("The state flag is marked inconsistent.")
    }
    /// A disk header parsing error.
    Parse(err: ParseError) {
        from()
        description("Disk header parsing error")
        display("Disk header parsing error: {}", err)
    }
    /// A disk error.
    Disk(err: disk::Error) {
        from()
        description("Disk I/O error")
        display("Disk I/O error: {}", err)
    }
}

impl<D: Disk> Driver<D> {
    /// Set up the driver from some disk.
    ///
    /// This will load the disk header and construct the driver. It will also set the disk to be in
    /// open state.
    fn open(disk: D, password: &[u8]) -> Result<Driver<D>, OpenError> {
        // Load the disk header into some buffer.
        let mut header_buf = [0; disk::SECTOR_SIZE];
        disk.read(0, &mut header_buf)?;

        // Decode the disk header.
        let mut header = DiskHeader::decode(header_buf)?;

        // TODO: Throw a warning if the flag is still in loading state.
        match header.consistency_flag {
            // Set the state flag to open.
            StateFlag::Closed => header.consistency_flag = StateFlag::Open,
            // The state inconsistent; throw an error.
            StateFlag::Inconsistent => return Err(OpenError::InconsistentState),
        }

        // Update the version.
        header.version_number = VERSION_NUMBER;

        // Construct the driver.
        let mut driver = Driver {
            // Generate the cipher (key, configuration etc.) from the disk header.
            cipher: crypto::Cipher(header.cipher, password),
            header: header,
            disk: disk,
        };

        // Flush the updated header.
        driver.flush_header();

        Ok(driver)
    }

    /// Initialize the disk.
    ///
    /// This stores disk header and makes the disk ready for use, returning the driver.
    fn init(disk: D) -> Result<Driver<D>, disk::Error> {
        // Construct the driver.
        let mut driver = Driver {
            header: DiskHeader::default(),
            disk: disk,
        };

        // Flush the default header.
        driver.flush_header()?;

        Ok(driver)
    }

    /// Flush the stored disk header.
    fn flush_header(&mut self) -> Result<(), disk::Error> {
        // Encode and write it to the disk.
        self.disk.write(0, &self.header.encode())
    }
}

impl<D: Disk> Drop for Driver<D> {
    fn drop(&mut self) {
        // Set the state flag to close so we know that it was a proper shutdown.
        self.header.state_flag = StateFlag::Closed;
        // Flush the header.
        self.flush_header();
    }
}

impl<D: Disk> Disk for Driver<D> {
    fn number_of_sectors(&self) -> Sector {
        self.disk.number_of_sectors()
    }

    fn write(sector: Sector, offset: SectorOffset, buffer: &[u8]) -> Result<(), Error> {
        match self.header.cipher {
            // Encryption disabled; forward the call to the inner disk.
            &Cipher::Identity => self.disk.write(sector, offset, buffer),
            _ => unimplemented!(),
        }
    }
    fn read(sector: Sector, offset: SectorOffset, buffer: &mut [u8]) -> Result<(), Error> {
        match self.header.cipher {
            // Encryption disabled; forward the call to the inner disk.
            &Cipher::Identity => self.disk.read(sector, offset, buffer),
            _ => unimplemented!(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverse_identity() {
        let mut header = DiskHeader::default();
        assert_eq!(DiskHeader::decode(header.encode()).unwrap(), header);

        header.version_number = 1;
        assert_eq!(DiskHeader::decode(header.encode()).unwrap(), header);

        header.cipher = Cipher::Speck;
        assert_eq!(DiskHeader::decode(header.encode()).unwrap(), header);

        header.consistency_flag = StateFlag::Inconsistent;
        assert_eq!(DiskHeader::decode(header.encode()).unwrap(), header);

        header.state_block_address = 500;
        assert_eq!(DiskHeader::decode(header.encode()).unwrap(), header);
    }

    #[test]
    fn manual_mutation() {
        let mut header = DiskHeader::default();
        let mut sector = header.encode();

        header.magic_number = MagicNumber::PartialCompatibility;
        sector[7] = b'~';

        assert_eq!(sector, header.encode());

        header.version_number |= 0xFF;
        sector[8] = 0xFF;

        assert_eq!(sector, header.encode());

        header.cipher = Cipher::Speck;
        sector[64] = 1;
        sector[65] = !1;

        assert_eq!(sector, header.encode());

        header.encryption_parameters[0] = 52;
        sector[68] = 52;
        sector[84] = !52;

        assert_eq!(sector, header.encode());

        header.state_block_address |= 0xFF;
        sector[128] = 0xFF;

        assert_eq!(sector, header.encode());

        header.consistency_flag = StateFlag::Open;
        sector[144] = 1;
        sector[145] = !1;

        assert_eq!(sector, header.encode());
    }

    #[test]
    fn corrupt_extra() {
        let mut sector = DiskHeader::default().encode();
        sector[12] ^= 2;
        assert_eq!(DiskHeader::decode(sector), Err(Error::CorruptVersionNumber));

        let mut sector = DiskHeader::default().encode();
        sector[65] ^= 1;
        assert_eq!(DiskHeader::decode(sector), Err(Error::CorruptCipher));

        let mut sector = DiskHeader::default().encode();
        sector[84] ^= 1;
        assert_eq!(DiskHeader::decode(sector), Err(Error::CorruptEncryptionParameters));

        let mut sector = DiskHeader::default().encode();
        sector[128] ^= 1;
        assert_eq!(DiskHeader::decode(sector), Err(Error::CorruptStateBlockAddress));

        let mut sector = DiskHeader::default().encode();
        sector[145] ^= 1;
        assert_eq!(DiskHeader::decode(sector), Err(Error::CorruptStateFlag));
    }

    #[test]
    fn unknown_format() {
        let mut sector = DiskHeader::default().encode();
        sector[0] = b'A';
        assert_eq!(DiskHeader::decode(sector), Err(Error::UnknownFormat));
    }

    #[test]
    fn incompatible_version() {
        let mut sector = DiskHeader::default().encode();
        sector[11] = 0xFF;
        assert_eq!(DiskHeader::decode(sector), Err(Error::IncompatibleVersion));
    }

    #[test]
    fn wrong_cipher() {
        let mut sector = DiskHeader::default().encode();
        sector[64] = 0xFF;
        assert_eq!(DiskHeader::decode(sector), Err(Error::InvalidCipher));
        sector[65] = 0xFF;
        assert_eq!(DiskHeader::decode(sector), Err(Error::UnknownCipher));
    }

    #[test]
    fn unknown_consistency_flag() {
        let mut sector = DiskHeader::default().encode();
        sector[144] = 6;
        assert_eq!(DiskHeader::decode(sector), Err(Error::UnknownStateFlag));
    }
}
