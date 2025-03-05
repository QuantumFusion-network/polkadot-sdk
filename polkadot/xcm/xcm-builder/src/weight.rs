// Copyright (C) Parity Technologies (UK) Ltd.
// This file is part of Polkadot.

// Polkadot is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// Polkadot is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with Polkadot.  If not, see <http://www.gnu.org/licenses/>.

use codec::Decode;
use core::{marker::PhantomData, result::Result};
use frame_support::{
	dispatch::GetDispatchInfo,
	traits::{
		fungible::{Balanced, Credit, Inspect},
		Get, OnUnbalanced as OnUnbalancedT,
	},
	weights::{
		constants::{WEIGHT_PROOF_SIZE_PER_MB, WEIGHT_REF_TIME_PER_SECOND},
		WeightToFee as WeightToFeeT,
	},
};
use sp_runtime::traits::{SaturatedConversion, Saturating, Zero};
use xcm::latest::{prelude::*, GetWeight, Weight};
use xcm_executor::{
	traits::{WeightBounds, WeightTrader},
	AssetsInHolding,
};

pub struct FixedWeightBounds<T, C, M>(PhantomData<(T, C, M)>);
impl<T: Get<Weight>, C: Decode + GetDispatchInfo, M: Get<u32>> WeightBounds<C>
	for FixedWeightBounds<T, C, M>
{
	fn weight(message: &mut Xcm<C>) -> Result<Weight, ()> {
		tracing::trace!(target: "xcm::weight", ?message, "FixedWeightBounds");
		let mut instructions_left = M::get();
		Self::weight_with_limit(message, &mut instructions_left)
	}
	fn instr_weight(instruction: &mut Instruction<C>) -> Result<Weight, ()> {
		Self::instr_weight_with_limit(instruction, &mut u32::max_value())
	}
}

impl<T: Get<Weight>, C: Decode + GetDispatchInfo, M> FixedWeightBounds<T, C, M> {
	fn weight_with_limit(message: &mut Xcm<C>, instrs_limit: &mut u32) -> Result<Weight, ()> {
		let mut r: Weight = Weight::zero();
		*instrs_limit = instrs_limit.checked_sub(message.0.len() as u32).ok_or(())?;
		for instruction in message.0.iter_mut() {
			r = r
				.checked_add(&Self::instr_weight_with_limit(instruction, instrs_limit)?)
				.ok_or(())?;
		}
		Ok(r)
	}
	fn instr_weight_with_limit(
		instruction: &mut Instruction<C>,
		instrs_limit: &mut u32,
	) -> Result<Weight, ()> {
		let instr_weight = match instruction {
			Transact { ref mut call, .. } => call.ensure_decoded()?.get_dispatch_info().call_weight,
			SetErrorHandler(xcm) | SetAppendix(xcm) | ExecuteWithOrigin { xcm, .. } =>
				Self::weight_with_limit(xcm, instrs_limit)?,
			_ => Weight::zero(),
		};
		T::get().checked_add(&instr_weight).ok_or(())
	}
}

pub struct WeightInfoBounds<W, C, M>(PhantomData<(W, C, M)>);
impl<W, C, M> WeightBounds<C> for WeightInfoBounds<W, C, M>
where
	W: XcmWeightInfo<C>,
	C: Decode + GetDispatchInfo,
	M: Get<u32>,
	Instruction<C>: xcm::latest::GetWeight<W>,
{
	fn weight(message: &mut Xcm<C>) -> Result<Weight, ()> {
		tracing::trace!(target: "xcm::weight", ?message, "WeightInfoBounds");
		let mut instructions_left = M::get();
		Self::weight_with_limit(message, &mut instructions_left)
	}
	fn instr_weight(instruction: &mut Instruction<C>) -> Result<Weight, ()> {
		Self::instr_weight_with_limit(instruction, &mut u32::max_value())
	}
}

impl<W, C, M> WeightInfoBounds<W, C, M>
where
	W: XcmWeightInfo<C>,
	C: Decode + GetDispatchInfo,
	M: Get<u32>,
	Instruction<C>: xcm::latest::GetWeight<W>,
{
	fn weight_with_limit(message: &mut Xcm<C>, instrs_limit: &mut u32) -> Result<Weight, ()> {
		let mut r: Weight = Weight::zero();
		*instrs_limit = instrs_limit.checked_sub(message.0.len() as u32).ok_or(())?;
		for instruction in message.0.iter_mut() {
			r = r
				.checked_add(&Self::instr_weight_with_limit(instruction, instrs_limit)?)
				.ok_or(())?;
		}
		Ok(r)
	}
	fn instr_weight_with_limit(
		instruction: &mut Instruction<C>,
		instrs_limit: &mut u32,
	) -> Result<Weight, ()> {
		let instr_weight = match instruction {
			Transact { ref mut call, .. } => call.ensure_decoded()?.get_dispatch_info().call_weight,
			SetErrorHandler(xcm) | SetAppendix(xcm) => Self::weight_with_limit(xcm, instrs_limit)?,
			_ => Weight::zero(),
		};
		instruction.weight().checked_add(&instr_weight).ok_or(())
	}
}

/// Function trait for handling some revenue. Similar to a negative imbalance (credit) handler, but
/// for a `Asset`. Sensible implementations will deposit the asset in some known treasury or
/// block-author account.
pub trait TakeRevenue {
	/// Do something with the given `revenue`, which is a single non-wildcard `Asset`.
	fn take_revenue(revenue: Asset);
}

/// Null implementation just burns the revenue.
impl TakeRevenue for () {
	fn take_revenue(_revenue: Asset) {}
}

/// Simple fee calculator that requires payment in a single fungible at a fixed rate.
///
/// The constant `Get` type parameter should be the fungible ID, the amount of it required for one
/// second of weight and the amount required for 1 MB of proof.
pub struct FixedRateOfFungible<T: Get<(AssetId, u128, u128)>, R: TakeRevenue>(
	Weight,
	u128,
	PhantomData<(T, R)>,
);
impl<T: Get<(AssetId, u128, u128)>, R: TakeRevenue> WeightTrader for FixedRateOfFungible<T, R> {
	fn new() -> Self {
		Self(Weight::zero(), 0, PhantomData)
	}

	fn buy_weight(
		&mut self,
		weight: Weight,
		payment: AssetsInHolding,
		context: &XcmContext,
	) -> Result<AssetsInHolding, XcmError> {
		let (id, units_per_second, units_per_mb) = T::get();
		tracing::trace!(
			target: "xcm::weight",
			?id, ?weight, ?payment, ?context,
			"FixedRateOfFungible::buy_weight",
		);
		let amount = (units_per_second * (weight.ref_time() as u128) /
			(WEIGHT_REF_TIME_PER_SECOND as u128)) +
			(units_per_mb * (weight.proof_size() as u128) / (WEIGHT_PROOF_SIZE_PER_MB as u128));
		if amount == 0 {
			return Ok(payment)
		}
		let unused = payment.checked_sub((id, amount).into()).map_err(|error| {
			tracing::error!(target: "xcm::weight", ?amount, ?error, "FixedRateOfFungible::buy_weight Failed to substract from payment");
			XcmError::TooExpensive
		})?;
		self.0 = self.0.saturating_add(weight);
		self.1 = self.1.saturating_add(amount);
		Ok(unused)
	}

	fn refund_weight(&mut self, weight: Weight, context: &XcmContext) -> Option<Asset> {
		let (id, units_per_second, units_per_mb) = T::get();
		tracing::trace!(target: "xcm::weight", ?id, ?weight, ?context, "FixedRateOfFungible::refund_weight");
		let weight = weight.min(self.0);
		let amount = (units_per_second * (weight.ref_time() as u128) /
			(WEIGHT_REF_TIME_PER_SECOND as u128)) +
			(units_per_mb * (weight.proof_size() as u128) / (WEIGHT_PROOF_SIZE_PER_MB as u128));
		self.0 -= weight;
		self.1 = self.1.saturating_sub(amount);
		if amount > 0 {
			Some((id, amount).into())
		} else {
			None
		}
	}
}

impl<T: Get<(AssetId, u128, u128)>, R: TakeRevenue> Drop for FixedRateOfFungible<T, R> {
	fn drop(&mut self) {
		if self.1 > 0 {
			R::take_revenue((T::get().0, self.1).into());
		}
	}
}

/// Weight trader which uses the configured `WeightToFee` to set the right price for weight and then
/// places any weight bought into the right account.
pub struct UsingComponents<
	WeightToFee: WeightToFeeT<Balance = <Fungible as Inspect<AccountId>>::Balance>,
	AssetIdValue: Get<Location>,
	AccountId,
	Fungible: Balanced<AccountId> + Inspect<AccountId>,
	OnUnbalanced: OnUnbalancedT<Credit<AccountId, Fungible>>,
>(
	Weight,
	Fungible::Balance,
	PhantomData<(WeightToFee, AssetIdValue, AccountId, Fungible, OnUnbalanced)>,
);
impl<
		WeightToFee: WeightToFeeT<Balance = <Fungible as Inspect<AccountId>>::Balance>,
		AssetIdValue: Get<Location>,
		AccountId,
		Fungible: Balanced<AccountId> + Inspect<AccountId>,
		OnUnbalanced: OnUnbalancedT<Credit<AccountId, Fungible>>,
	> WeightTrader for UsingComponents<WeightToFee, AssetIdValue, AccountId, Fungible, OnUnbalanced>
{
	fn new() -> Self {
		Self(Weight::zero(), Zero::zero(), PhantomData)
	}

	fn buy_weight(
		&mut self,
		weight: Weight,
		payment: AssetsInHolding,
		context: &XcmContext,
	) -> Result<AssetsInHolding, XcmError> {
		tracing::trace!(target: "xcm::weight", ?weight, ?payment, ?context, "UsingComponents::buy_weight");
		let amount = WeightToFee::weight_to_fee(&weight);
		let u128_amount: u128 = amount.try_into().map_err(|_| {
			tracing::debug!(target: "xcm::weight", ?amount, "Weight fee could not be converted");
			XcmError::Overflow
		})?;
		let required = Asset { id: AssetId(AssetIdValue::get()), fun: Fungible(u128_amount) };
		let unused = payment.checked_sub(required).map_err(|error| {
			tracing::debug!(target: "xcm::weight", ?error, "Failed to substract from payment");
			XcmError::TooExpensive
		})?;
		self.0 = self.0.saturating_add(weight);
		self.1 = self.1.saturating_add(amount);
		Ok(unused)
	}

	fn refund_weight(&mut self, weight: Weight, context: &XcmContext) -> Option<Asset> {
		tracing::trace!(target: "xcm::weight", ?weight, ?context, available_weight = ?self.0, available_amount = ?self.1, "UsingComponents::refund_weight");
		let weight = weight.min(self.0);
		let amount = WeightToFee::weight_to_fee(&weight);
		self.0 -= weight;
		self.1 = self.1.saturating_sub(amount);
		let amount: u128 = amount.saturated_into();
		tracing::trace!(target: "xcm::weight", ?amount, "UsingComponents::refund_weight");
		if amount > 0 {
			Some((AssetIdValue::get(), amount).into())
		} else {
			None
		}
	}
}
impl<
		WeightToFee: WeightToFeeT<Balance = <Fungible as Inspect<AccountId>>::Balance>,
		AssetId: Get<Location>,
		AccountId,
		Fungible: Balanced<AccountId> + Inspect<AccountId>,
		OnUnbalanced: OnUnbalancedT<Credit<AccountId, Fungible>>,
	> Drop for UsingComponents<WeightToFee, AssetId, AccountId, Fungible, OnUnbalanced>
{
	fn drop(&mut self) {
		OnUnbalanced::on_unbalanced(Fungible::issue(self.1));
	}
}
