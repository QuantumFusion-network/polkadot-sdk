//! # Template Pallet
//!
//! A pallet with minimal functionality to help developers understand the essential components of
//! writing a FRAME pallet. It is typically used in beginner tutorials or in Polkadot SDK template
//! as a starting point for creating a new pallet and **not meant to be used in production**.
//!
//! ## Overview
//!
//! This template pallet contains basic examples of:
//! - declaring a storage item that stores a single block-number
//! - declaring and using events
//! - declaring and using errors
//! - a dispatchable function that allows a user to set a new value to storage and emits an event
//!   upon success
//! - another dispatchable function that causes a custom error to be thrown
//!
//! Each pallet section is annotated with an attribute using the `#[pallet::...]` procedural macro.
//! This macro generates the necessary code for a pallet to be aggregated into a FRAME runtime.
//!
//! To get started with pallet development, consider using this tutorial:
//!
//! <https://paritytech.github.io/polkadot-sdk/master/polkadot_sdk_docs/guides/your_first_pallet/index.html>
//!
//! And reading the main documentation of the `frame` crate:
//!
//! <https://paritytech.github.io/polkadot-sdk/master/polkadot_sdk_docs/polkadot_sdk/frame_runtime/index.html>
//!
//! And looking at the frame [`kitchen-sink`](https://paritytech.github.io/polkadot-sdk/master/pallet_example_kitchensink/index.html)
//! pallet, a showcase of all pallet macros.
//!
//! ### Pallet Sections
//!
//! The pallet sections in this template are:
//!
//! - A **configuration trait** that defines the types and parameters which the pallet depends on
//!   (denoted by the `#[pallet::config]` attribute). See: [`Config`].
//! - A **means to store pallet-specific data** (denoted by the `#[pallet::storage]` attribute).
//!   See: [`storage_types`].
//! - A **declaration of the events** this pallet emits (denoted by the `#[pallet::event]`
//!   attribute). See: [`Event`].
//! - A **declaration of the errors** that this pallet can throw (denoted by the `#[pallet::error]`
//!   attribute). See: [`Error`].
//! - A **set of dispatchable functions** that define the pallet's functionality (denoted by the
//!   `#[pallet::call]` attribute). See: [`dispatchables`].
//!
//! Run `cargo doc --package pallet-template --open` to view this pallet's documentation.

#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

const LOG_TARGET: &str = "runtime::template";

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

pub mod weights;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;

// <https://paritytech.github.io/polkadot-sdk/master/polkadot_sdk_docs/polkadot_sdk/frame_runtime/index.html>
// <https://paritytech.github.io/polkadot-sdk/master/polkadot_sdk_docs/guides/your_first_pallet/index.html>
//
// To see a full list of `pallet` macros and their use cases, see:
// <https://paritytech.github.io/polkadot-sdk/master/pallet_example_kitchensink/index.html>
// <https://paritytech.github.io/polkadot-sdk/master/frame_support/pallet_macros/index.html>
#[frame::pallet]
pub mod pallet {
	use frame::prelude::*;

	/// Configure the pallet by specifying the parameters and types on which it depends.
	#[pallet::config]
	pub trait Config: frame_system::Config {
		type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

		/// A type representing the weights required by the dispatchables of this pallet.
		type WeightInfo: crate::weights::WeightInfo;

		#[pallet::constant]
		type TimeoutBlocks: Get<u32>; // BlockNumberFor<T>

		#[pallet::constant]
		type CoolDownPeriodBlocks: Get<u32>; // BlockNumberFor<T>
	}

	#[pallet::pallet]
	pub struct Pallet<T>(_);

	#[derive(
		Encode, Decode, MaxEncodedLen, TypeInfo, CloneNoBound, PartialEqNoBound, DefaultNoBound,
	)]
	#[scale_info(skip_type_params(T))]
	pub enum SlowchainState<T: Config> {
		#[default]
		Operational {
			last_alive_message_block_number: BlockNumberFor<T>,
		},
		CoolDown {
			start_block_number: BlockNumberFor<T>,
		},
		SlowMode {
			start_block_number: BlockNumberFor<T>,
		},
	}

	#[pallet::storage]
	pub type State<T: Config> = StorageValue<_, SlowchainState<T>, ValueQuery>;

	#[pallet::storage]
	pub type ValidatorSet<T: Config> =
		StorageMap<_, Blake2_128Concat, T::AccountId, (), ValueQuery>;

	/// Pallets use events to inform users when important changes are made.
	/// <https://paritytech.github.io/polkadot-sdk/master/polkadot_sdk_docs/guides/your_first_pallet/index.html#event-and-error>
	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		Heartbeat {
			block_number: BlockNumberFor<T>,
			who: T::AccountId,
		},
		StartedCoolDown {
			block_number: BlockNumberFor<T>,
			last_alive_message_block_number: BlockNumberFor<T>,
		},
		FinishedCoolDown {
			block_number: BlockNumberFor<T>,
		},
	}

	/// Errors inform users that something went wrong.
	/// <https://paritytech.github.io/polkadot-sdk/master/polkadot_sdk_docs/guides/your_first_pallet/index.html#event-and-error>
	#[pallet::error]
	pub enum Error<T> {
		BlockTimeout,
		CoolDownPeriod,
	}

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
		fn on_initialize(bn: BlockNumberFor<T>) -> Weight {
			let current_block_number = bn;
			let current_block_number_: U256 = current_block_number.into();
			log::info!(
				target: crate::LOG_TARGET,
				"on_initialize called, block {}", current_block_number_
			);

			match <State<T>>::get() {
				SlowchainState::Operational { last_alive_message_block_number } => {
					let last_alive_message_block_number_bn: U256 =
						last_alive_message_block_number.into();
					let timeout_blocks: U256 = T::TimeoutBlocks::get().into();

					let deadline_block_number: BlockNumberFor<T> =
						match (last_alive_message_block_number_bn + timeout_blocks).try_into() {
							Ok(n) => n,
							Err(_e) => panic!(), // handle
						};

					if current_block_number > deadline_block_number {
						log::info!(
							target: crate::LOG_TARGET,
							"on_initialize: alive message deadline exceeded. Starting cool down"
						);
						<State<T>>::put(SlowchainState::CoolDown {
							start_block_number: current_block_number,
						});
						Self::deposit_event(Event::StartedCoolDown {
							block_number: current_block_number,
							last_alive_message_block_number,
						});
					}
				},
				SlowchainState::CoolDown { start_block_number } => {
					let start_block_number: U256 = start_block_number.into();
					let cool_down_period_blocks: U256 = T::CoolDownPeriodBlocks::get().into();
					let cool_down_period_deadline_block_number: BlockNumberFor<T> =
						match (start_block_number + cool_down_period_blocks).try_into() {
							Ok(n) => n,
							Err(_e) => panic!(),
						};
					if current_block_number > cool_down_period_deadline_block_number {
						<State<T>>::put(SlowchainState::Operational {
							last_alive_message_block_number: current_block_number,
						});
						Self::deposit_event(Event::FinishedCoolDown {
							block_number: current_block_number,
						});
					}
				},
				_ => (),
			}

			Weight::zero() // count storage weight
		}
	}

	/// Dispatchable functions allows users to interact with the pallet and invoke state changes.
	/// These functions materialize as "extrinsics", which are often compared to transactions.
	/// Dispatchable functions must be annotated with a weight and must return a DispatchResult.
	/// <https://paritytech.github.io/polkadot-sdk/master/polkadot_sdk_docs/guides/your_first_pallet/index.html#dispatchables>
	#[pallet::call]
	impl<T: Config> Pallet<T> {
		#[pallet::call_index(0)]
		#[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().reads_writes(1,1))]
		pub fn handle_alive_message(origin: OriginFor<T>) -> DispatchResultWithPostInfo {
			let who = ensure_signed(origin)?;

			let current_block_number = frame_system::Pallet::<T>::block_number();

			match <State<T>>::get() {
				SlowchainState::Operational { last_alive_message_block_number } => {
					// i'm online pallet
					ensure!(current_block_number > last_alive_message_block_number, "BlockNumberDecreased"); // '>=' ?
					<State<T>>::put(SlowchainState::Operational {
						last_alive_message_block_number: current_block_number,
					});
					Self::deposit_event(Event::Heartbeat {
						block_number: current_block_number,
						who,
					});
				},
				SlowchainState::CoolDown { start_block_number } => {
					let start_block_number: U256 = start_block_number.into();
					let cool_down_period_blocks: U256 = T::CoolDownPeriodBlocks::get().into();
					let cool_down_period_deadline_block_number: BlockNumberFor<T> =
						match (start_block_number + cool_down_period_blocks).try_into() {
							Ok(n) => n,
							Err(_e) => panic!(),
						};
					if current_block_number > cool_down_period_deadline_block_number {
						<State<T>>::put(SlowchainState::Operational {
							last_alive_message_block_number: current_block_number,
						});
						Self::deposit_event(Event::FinishedCoolDown {
							block_number: current_block_number,
						});
					} else {
						return Err(Error::<T>::CoolDownPeriod.into());
					}
				},
				SlowchainState::SlowMode { start_block_number: _ } => unimplemented!("SlowMode"),
			}

			Ok(().into())
		}
	}
}
