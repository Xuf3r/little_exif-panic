// Copyright © 2024 Tobias J. Prisching <tobias.prisching@icloud.com> and CONTRIBUTORS
// See https://github.com/TechnikTobi/little_exif#license for licensing details

use std::path::Path;
use std::io::Read;
use std::io::Write;
use std::io::Seek;
use std::io::SeekFrom;
use std::fs::File;
use std::fs::OpenOptions;

use crate::endian::*;
use crate::general_file_io::*;

pub(crate) const JPG_SIGNATURE: [u8; 2] = [0xff, 0xd8];

const JPG_MARKER_PREFIX: u8  = 0xff;
const JPG_APP1_MARKER:   u16 = 0xffe1;

fn
encode_metadata_jpg
(
	exif_vec: &Vec<u8>
)
-> Vec<u8>
{
	// vector storing the data that will be returned
	let mut jpg_exif: Vec<u8> = Vec::new();

	// Compute the length of the exif data (includes the two bytes of the
	// actual length field)
	let length = 2u16 + (EXIF_HEADER.len() as u16) + (exif_vec.len() as u16);

	// Start with the APP1 marker and the length of the data
	// Then copy the previously encoded EXIF data 
	jpg_exif.extend(to_u8_vec_macro!(u16, &JPG_APP1_MARKER, &Endian::Big));
	jpg_exif.extend(to_u8_vec_macro!(u16, &length, &Endian::Big));
	jpg_exif.extend(EXIF_HEADER.iter());
	jpg_exif.extend(exif_vec.iter());

	return jpg_exif;
}

fn
check_signature
(
	path: &Path
)
-> Result<File, std::io::Error>
{
	if !path.exists()
	{
		return io_error!(NotFound, "Can't open JPG file - File does not exist!");
	}

	let mut file = OpenOptions::new()
		.read(true)
		.write(true)
		.open(path)
		.expect("Could not open file");
	
	// Check the signature
	let mut signature_buffer = [0u8; 2];
	file.read(&mut signature_buffer).unwrap();
	let signature_is_valid = signature_buffer.iter()
		.zip(JPG_SIGNATURE.iter())
		.filter(|&(read, constant)| read == constant)
		.count() == JPG_SIGNATURE.len();

	if !signature_is_valid
	{
		return io_error!(InvalidData, "Can't open JPG file - Wrong signature!");
	}

	// Signature is valid - can proceed using the file as JPG file
	return Ok(file);
}

pub(crate) fn
clear_metadata
(
	path: &Path
)
-> Result<u8, std::io::Error>
{
	let file_result = check_signature(path); // i honestly have no idea: it only removes app1

	if file_result.is_err()
	{
		return Err(file_result.err().unwrap());
	}

	let mut full_file_buf:Vec<u8> = Vec::new();
	// Setup of variables necessary for going through the file
	let mut file = file_result.unwrap();                                        // The struct for interacting with the file

	file.read_to_end(&mut full_file_buf)?; // Here we load the file into memory instead

	let mut seek_counter = 2u64;                                                // A counter for keeping track of where in the file we currently are
	let mut byte_buffer = [0u8; 1];                                             // A buffer for reading in a byte of data from the file
	let mut previous_byte_was_marker_prefix = false;                            // A boolean for remembering if the previous byte was a marker prefix (0xFF)
	let mut cleared_segments: u8 = 0;                                           // A counter for keeping track of how many segments were cleared

	let mut iterator_file = full_file_buf.iter(); // We instantiate the iterator outside the loop scope to access it during the walk

	loop
	{
		// Read next byte into buffer
		if let Some(byte) = iterator_file.next() {
				byte_buffer[0] = byte.clone()
			}

		if previous_byte_was_marker_prefix
		{
			println!("{:?}", &byte_buffer[0]);
			match byte_buffer[0]
			{
				0xe1	=> {
					// APP1 marker

					// Read in the length of the segment
					// (which follows immediately after the marker)
					let mut length_buffer = [0u8; 2];

					if let (Some(&byte1), Some(&byte2)) = (iterator_file.next(), iterator_file.next()) {
						length_buffer = [byte1, byte2];
					}

					// Decode the length to determine how much more data there is
					let length = from_u8_vec_macro!(u16, &length_buffer.to_vec(), &Endian::Big);
					let remaining_length = length - 2;


					if remaining_length > 0 {
						iterator_file.nth((remaining_length - 1) as usize); // What if it's not? surely it is
					} else {
						unreachable!("If rem_len is <= 0 then it's not a valid JPEG\
						 - it must have at least a single SOS after APP1")
					}


					// ...copy data from there onwards into a buffer...
					let mut full_temp = full_file_buf.clone();
					let (_, buffer) = full_temp.split_at_mut((seek_counter as usize) + (remaining_length as usize ) - 1);   // here we just copy the data

					let buffer: Vec<u8> = buffer.to_vec();

					// This essentially shits the right-most bytes n bytes to the left
					full_file_buf[((seek_counter as usize)- 1)..][..buffer.len()].copy_from_slice(&buffer);


					iterator_file = full_file_buf.iter(); // Here we reassign the iterator to represent the mutated bytestream
					iterator_file.nth((seek_counter - 1) as usize); // We walk to the previous position immediately
															// This allows to simply next()-walk bytes outside.

					seek_counter -= 2;
					cleared_segments += 1;

				},
				0xd9	=> break,                                               // EOI marker
				_		=> (),                                                  // Every other marker
			}

			previous_byte_was_marker_prefix = false;
		}
		else
		{
			previous_byte_was_marker_prefix = byte_buffer[0] == JPG_MARKER_PREFIX;
		}

		seek_counter += 1;

	}
	// check if we can save the file
	// probably possible to optimize further by returning the purged bytestream itself
	if let Err(_) = file.write(&*full_file_buf) {
		return io_error!(Other, "Couldn't save JPEG file!");
	}


	return Ok(cleared_segments);
}

/// Provides the JPEG specific encoding result as vector of bytes to be used
/// by the user (e.g. in combination with another library)
pub(crate) fn
as_u8_vec
(
	general_encoded_metadata: &Vec<u8>
)
-> Vec<u8>
{
	encode_metadata_jpg(general_encoded_metadata)
}

/// Writes the given generally encoded metadata to the JP(E)G image file at 
/// the specified path. 
/// Note that any previously stored metadata under the APP1 marker gets removed
/// first before writing the "new" metadata. 
pub(crate) fn
write_metadata
(
	path: &Path,
	general_encoded_metadata: &Vec<u8>
)
-> Result<(), std::io::Error>
{
	clear_metadata(path)?;

	// Encode the data specifically for JPG and open the file...
	let encoded_metadata = encode_metadata_jpg(general_encoded_metadata);
	let mut file = OpenOptions::new()
		.write(true)
		.read(true)
		.open(path)
		.expect("Could not open file");

	// ...and copy everything after the signature into a buffer...
	let mut buffer = Vec::new();
	perform_file_action!(file.seek(SeekFrom::Start(JPG_SIGNATURE.len() as u64)));
	perform_file_action!(file.read_to_end(&mut buffer));

	// ...seek back to where the encoded data will be written
	perform_file_action!(file.seek(SeekFrom::Start(JPG_SIGNATURE.len() as u64)));

	// ...and write the exif data...
	perform_file_action!(file.write_all(&encoded_metadata));

	// ...and the rest of the file from the buffer
	perform_file_action!(file.write_all(&buffer));
	
	return Ok(());
}

pub(crate) fn
read_metadata
(
	path: &Path
)
-> Result<Vec<u8>, std::io::Error>
{
	let file_result = check_signature(path);

	if file_result.is_err()
	{
		return Err(file_result.err().unwrap());
	}

	// Setup of variables necessary for going through the file
	let mut file = file_result.unwrap();                                        // The struct for interacting with the file
	let mut byte_buffer = [0u8; 1];                                             // A buffer for reading in a byte of data from the file
	let mut previous_byte_was_marker_prefix = false;                            // A boolean for remembering if the previous byte was a marker prefix (0xFF)

	loop
	{
		// Read next byte into buffer
		perform_file_action!(file.read(&mut byte_buffer));

		if previous_byte_was_marker_prefix
		{
			match byte_buffer[0]
			{
				0xe1	=> {                                                    // APP1 marker

					// Read in the length of the segment
					// (which follows immediately after the marker)
					let mut length_buffer = [0u8; 2];
					perform_file_action!(file.read(&mut length_buffer));

					// Decode the length to determine how much more data there is
					let length = from_u8_vec_macro!(u16, &length_buffer.to_vec(), &Endian::Big);
					let remaining_length = (length - 2) as usize;

					// Read in the remaining data
					let mut buffer = vec![0u8; remaining_length];
					perform_file_action!(file.read(&mut buffer));

					return Ok(buffer);
				},
				0xd9	=> break,                                               // EOI marker
				_		=> (),                                                  // Every other marker
			}

			previous_byte_was_marker_prefix = false;
		}
		else
		{
			previous_byte_was_marker_prefix = byte_buffer[0] == JPG_MARKER_PREFIX;
		}
	}

	return io_error!(Other, "No EXIF data found!");
}