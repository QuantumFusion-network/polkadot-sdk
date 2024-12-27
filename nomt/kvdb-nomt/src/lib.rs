use anyhow::Result;
use kvdb::{DBKeyValue, DBOp, DBTransaction, DBValue, KeyValueDB};
use nomt::{KeyReadWrite, Node, Nomt, Options, Witness, WitnessedOperations};
use sha2::Digest;

use std::{
	cmp,
	collections::HashMap,
	error, io,
	path::{Path, PathBuf}, rc::Rc,
};

const NOMT_DB_FOLDER: &str = "/Users/max/workspace/Rust/Profiterole/polkadot-sdk/kvdb_nomt_db_data";

#[cfg(target_os = "linux")]
use regex::Regex;
#[cfg(target_os = "linux")]
use std::fs::File;
#[cfg(target_os = "linux")]
use std::process::Command;

fn other_io_err<E>(e: E) -> io::Error
where
	E: Into<Box<dyn error::Error + Send + Sync>>,
{
	io::Error::new(io::ErrorKind::Other, e)
}

fn invalid_column(col: u32) -> io::Error {
	other_io_err(format!("No such column family: {:?}", col))
}

pub struct NomtDB {
	nomt: Nomt,
}

impl Default for NomtDB {
	fn default() -> Self {
		// Define the options used to open NOMT
		let mut opts = Options::new();
		opts.path(NOMT_DB_FOLDER);
		opts.commit_concurrency(1);
		// Open NOMT database, it will create the folder if it does not exist
		// let nomt = Nomt::open(opts);
		match Nomt::open(opts) {
			Ok(nomt) => Self { nomt },
			Err(e) => {
				eprintln!("Error opening NOMT database: {:?}", e);
				std::process::exit(1);
			}
		}
	}
}

impl NomtDB {
	pub fn open(path: &Path) -> Result<NomtDB> {
		let mut opts = Options::new();
		opts.path(&path);
		opts.commit_concurrency(1);
		// Open NOMT database, it will create the folder if it does not exist
		// let nomt = Nomt::open(opts);
		match Nomt::open(opts) {
			Ok(nomt) => Ok(Self { nomt }),
			Err(e) => {
				eprintln!("Error opening NOMT database: {:?}", e);
				std::process::exit(1);
			}
		}
	}

	pub fn from_opts(opts: Options) -> Result<NomtDB> {
		// Open NOMT database, it will create the folder if it does not exist
		let nomt: Nomt = Nomt::open(opts)?;
		Ok(Self { nomt })
	}
}

impl KeyValueDB for NomtDB {

	fn get(&self, col: u32, key: &[u8]) -> io::Result<Option<DBValue>> {
		let mut session: nomt::Session = self.nomt.begin_session();

		// Reading a key from the database
		let key_path = sha2::Sha256::digest(key).into();
		// let _value = session.tentative_read_slot(key_path)?;
		match session.tentative_read_slot(key_path) {
			Err(_) => Err(invalid_column(col)),
			Ok(Some(value)) => {
				let value = value.to_vec();
				Ok(Some(DBValue::from(value)))
			},
			Ok(None) => Ok(None),
		}
	}

	fn get_by_prefix(&self, col: u32, prefix: &[u8]) -> io::Result<Option<DBValue>> {
		eprintln!("get_by_prefix is not implemented yet.. soon later..");
		Ok(None)
	}

	fn write(&self, transaction: DBTransaction) -> io::Result<()> {
		let mut session = self.nomt.begin_session();

		// To commit the batch to the backend we need to collect every
		// performed actions into a vector where items are ordered by the key_path
		let mut actual_access: Vec<_> = vec![
			// (key_path_1, KeyReadWrite::ReadThenWrite(value.clone(), None)),
			// (key_path_2, KeyReadWrite::Write(value)),
		];
		let prev_root = self.nomt.root();

		let ops = transaction.ops;
		for op in ops {
			match op {
				DBOp::Insert { col, key, value } =>
					{
						let key_path = sha2::Sha256::digest(key).into();
						let v: Option<Rc<Vec<u8>>> = Some(Rc::new(value));
						session.tentative_write_slot(key_path);
						actual_access.push((key_path, KeyReadWrite::Write(v)));
					},
				DBOp::Delete { col, key } =>
					{
						let key_path = sha2::Sha256::digest(key).into();
						session.tentative_write_slot(key_path);
						actual_access.push((key_path, KeyReadWrite::Write(None)));
					},
				DBOp::DeletePrefix { col, prefix } =>
					eprintln!("Prefix not awailable yet.."),
			}
		}
		// Зачем-то происходит сортировка по ключу
		// Без сортировки вообще не едет ))
		actual_access.sort_by_key(|(k, _)| *k);
		let err = self.nomt.commit_and_prove(session, actual_access);
		match err {
			Ok(root) => {
				let (root, witness, witnessed) = root;
				Ok(())
			},
			Err(e) => {
				eprintln!("Error committing batch: {:?}", e);
				Err(io::Error::new(io::ErrorKind::Other, e))
			},
		}
	}

	fn iter<'a>(&'a self, col: u32) -> Box<dyn Iterator<Item = io::Result<DBKeyValue>> + 'a> {
		// match self.columns.read().get(&col) {
		// 	Some(map) => Box::new(
		// 		// TODO: worth optimizing at all?
		// 		map.clone().into_iter().map(|(k, v)| Ok((k.into(), v))),
		// 	),
		// 	None => Box::new(std::iter::once(Err(invalid_column(col)))),
		// }
		Box::new(std::iter::once(Err(invalid_column(col))))
	}

	fn iter_with_prefix<'a>(
		&'a self,
		col: u32,
		prefix: &'a [u8],
	) -> Box<dyn Iterator<Item = io::Result<DBKeyValue>> + 'a> {
		// match self.columns.read().get(&col) {
		// 	Some(map) => Box::new(
		// 		map.clone()
		// 			.into_iter()
		// 			.filter(move |&(ref k, _)| k.starts_with(prefix))
		// 			.map(|(k, v)| Ok((k.into(), v))),
		// 	),
		// 	None => Box::new(std::iter::once(Err(invalid_column(col)))),
		// }
		Box::new(std::iter::once(Err(invalid_column(col))))
	}

}
