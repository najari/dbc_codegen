/// The name of the DBC file this code was generated from
#[allow(dead_code)]
pub const DBC_FILE_NAME: &str = "example.dbc";
/// The version of the DBC file this code was generated from
#[allow(dead_code)]
pub const DBC_FILE_VERSION: &str = "FanSimulationExample";
#[allow(unused_imports)]
use core::ops::BitOr;
#[allow(unused_imports)]
use bitvec::prelude::*;
#[allow(unused_imports)]
use embedded_can::{Id, StandardId, ExtendedId};
/// All messages
#[allow(
    clippy::absurd_extreme_comparisons,
    clippy::excessive_precision,
    clippy::manual_range_contains,
    clippy::unnecessary_cast,
    clippy::useless_conversion,
    unused_comparisons,
    unused_variables,
)]
#[derive(Clone)]
pub enum Messages {
    /// TemperatureData
    TemperatureData(TemperatureData),
    /// FanControl
    FanControl(FanControl),
    /// FanStatus
    FanStatus(FanStatus),
}
#[allow(
    clippy::absurd_extreme_comparisons,
    clippy::excessive_precision,
    clippy::manual_range_contains,
    clippy::unnecessary_cast,
    clippy::useless_conversion,
    unused_comparisons,
    unused_variables,
)]
impl Messages {
    /// Read message from CAN frame
    #[inline(never)]
    pub fn from_can_message(id: Id, payload: &[u8]) -> Result<Self, CanError> {
        let res = match id {
            TemperatureData::MESSAGE_ID => {
                Messages::TemperatureData(TemperatureData::try_from(payload)?)
            }
            FanControl::MESSAGE_ID => {
                Messages::FanControl(FanControl::try_from(payload)?)
            }
            FanStatus::MESSAGE_ID => Messages::FanStatus(FanStatus::try_from(payload)?),
            id => return Err(CanError::UnknownMessageId(id)),
        };
        Ok(res)
    }
}
/// TemperatureData
///
/// - Standard ID: 256 (0x100)
/// - Size: 8 bytes
/// - Transmitter: ClimateController
///
/// Provides the temperature input used to calculate the target fan speed.
#[derive(Clone, Copy)]
pub struct TemperatureData {
    raw: [u8; 8],
}
#[allow(
    clippy::absurd_extreme_comparisons,
    clippy::excessive_precision,
    clippy::manual_range_contains,
    clippy::unnecessary_cast,
    clippy::useless_conversion,
    unused_comparisons,
    unused_variables,
)]
impl TemperatureData {
    pub const MESSAGE_ID: embedded_can::Id = Id::Standard(
        StandardId::new(0x100).unwrap(),
    );
    pub const MESSAGE_SIZE: usize = 8;
    pub const AVERAGE_TEMPERATURE_MIN: f64 = -40_f64;
    pub const AVERAGE_TEMPERATURE_MAX: f64 = 215.3_f64;
    /// Constructs a new `TemperatureData` message from values.
    pub fn new(average_temperature: f64) -> Result<Self, CanError> {
        let mut res = Self { raw: [0x00; 8] };
        res.set_average_temperature(average_temperature)?;
        Ok(res)
    }
    /// Returns the raw `TemperatureData` message payload.
    pub fn raw(&self) -> &[u8; 8] {
        &self.raw
    }
    /// Physical value of original signal "AverageTemperature".
    pub fn average_temperature(&self) -> f64 {
        ((self.average_temperature_raw_val() as f64) * 0.1_f64 + -40_f64) as f64
    }
    /// Returns the raw value of `AverageTemperature`.
    ///
    /// - Start bit: 0
    /// - Signal size: 16 bits
    /// - Byte order: LittleEndian
    /// - Value type: Unsigned
    #[inline(always)]
    pub fn average_temperature_raw_val(&self) -> u16 {
        self.raw.view_bits::<Lsb0>()[0..16].load_le::<u16>()
    }
    /// Sets the raw value of `AverageTemperature`.
    #[inline(always)]
    pub fn set_average_temperature_raw_val(
        &mut self,
        value: u16,
    ) -> Result<(), CanError> {
        if (i128::from(value)) < 0_i128 || (i128::from(value)) > 65535_i128 {
            return Err(CanError::ParameterOutOfRange {
                message_id: TemperatureData::MESSAGE_ID,
            });
        }
        self.raw.view_bits_mut::<Lsb0>()[0..16].store_le(value);
        Ok(())
    }
    /// Sets a finite physical value. Failure leaves the payload unchanged.
    pub fn set_average_temperature(&mut self, value: f64) -> Result<(), CanError> {
        self.set_average_temperature_quantized(value).map(|_| ())
    }
    /// Sets a value and returns its actual quantized physical value.
    pub fn set_average_temperature_quantized(
        &mut self,
        value: f64,
    ) -> Result<f64, CanError> {
        if !value.is_finite() {
            return Err(CanError::ParameterOutOfRange {
                message_id: TemperatureData::MESSAGE_ID,
            });
        }
        if (value as f64) < -40_f64 || (value as f64) > 215.3_f64 {
            return Err(CanError::ParameterOutOfRange {
                message_id: TemperatureData::MESSAGE_ID,
            });
        }
        let scaled = (value as f64 - -40_f64) / 0.1_f64;
        if !scaled.is_finite() {
            return Err(CanError::ParameterOutOfRange {
                message_id: TemperatureData::MESSAGE_ID,
            });
        }
        if scaled < (0_f64 - 1.0) || scaled > (65536_f64 + 1.0) {
            return Err(CanError::ParameterOutOfRange {
                message_id: TemperatureData::MESSAGE_ID,
            });
        }
        let base = scaled as i128;
        let bits = base
            + if scaled - base as f64 >= 0.5 {
                1
            } else if scaled - base as f64 <= -0.5 {
                -1
            } else {
                0
            };
        if (bits) < 0_i128 || (bits) > 65535_i128 {
            return Err(CanError::ParameterOutOfRange {
                message_id: TemperatureData::MESSAGE_ID,
            });
        }
        let value = bits as u16;
        let actual = ((value as f64) * 0.1_f64 + -40_f64) as f64;
        if !actual.is_finite() {
            return Err(CanError::ParameterOutOfRange {
                message_id: TemperatureData::MESSAGE_ID,
            });
        }
        if (actual as f64) < -40_f64 || (actual as f64) > 215.3_f64 {
            return Err(CanError::ParameterOutOfRange {
                message_id: TemperatureData::MESSAGE_ID,
            });
        }
        self.raw.view_bits_mut::<Lsb0>()[0..16].store_le(value);
        Ok(actual)
    }
}
impl core::convert::TryFrom<&[u8]> for TemperatureData {
    type Error = CanError;
    #[inline(always)]
    fn try_from(payload: &[u8]) -> Result<Self, Self::Error> {
        if payload.len() != 8 {
            return Err(CanError::InvalidPayloadSize);
        }
        let mut raw = [0u8; 8];
        raw.copy_from_slice(&payload[..8]);
        Ok(Self { raw })
    }
}
impl embedded_can::Frame for TemperatureData {
    fn new(id: impl Into<Id>, data: &[u8]) -> Option<Self> {
        if id.into() != Self::MESSAGE_ID { None } else { data.try_into().ok() }
    }
    fn new_remote(_id: impl Into<Id>, _dlc: usize) -> Option<Self> {
        unimplemented!()
    }
    fn is_extended(&self) -> bool {
        match self.id() {
            Id::Standard(_) => false,
            Id::Extended(_) => true,
        }
    }
    fn is_remote_frame(&self) -> bool {
        false
    }
    fn id(&self) -> Id {
        Self::MESSAGE_ID
    }
    fn dlc(&self) -> usize {
        self.raw.len()
    }
    fn data(&self) -> &[u8] {
        &self.raw
    }
}
/// FanControl
///
/// - Standard ID: 257 (0x101)
/// - Size: 8 bytes
/// - Transmitter: HMI
///
/// Runtime control settings for the fan simulation.
#[derive(Clone, Copy)]
pub struct FanControl {
    raw: [u8; 8],
}
#[allow(
    clippy::absurd_extreme_comparisons,
    clippy::excessive_precision,
    clippy::manual_range_contains,
    clippy::unnecessary_cast,
    clippy::useless_conversion,
    unused_comparisons,
    unused_variables,
)]
impl FanControl {
    pub const MESSAGE_ID: embedded_can::Id = Id::Standard(
        StandardId::new(0x101).unwrap(),
    );
    pub const MESSAGE_SIZE: usize = 8;
    pub const FAN_MODE_MIN: i128 = 0_i128;
    pub const FAN_MODE_MAX: i128 = 2_i128;
    pub const TARGET_TEMPERATURE_MIN: f64 = 0_f64;
    pub const TARGET_TEMPERATURE_MAX: f64 = 100_f64;
    /// Constructs a new `FanControl` message from values.
    pub fn new(
        fan_mode: FanControlFanMode,
        target_temperature: f64,
    ) -> Result<Self, CanError> {
        let mut res = Self { raw: [0x00; 8] };
        res.set_fan_mode(fan_mode)?;
        res.set_target_temperature(target_temperature)?;
        Ok(res)
    }
    /// Returns the raw `FanControl` message payload.
    pub fn raw(&self) -> &[u8; 8] {
        &self.raw
    }
    /// Returns the value of `FanMode`.
    ///
    /// Fan control mode: Off, On, Auto.
    ///
    /// - Min: 0
    /// - Max: 2
    /// - Unit: Not specified
    /// - Receivers: FanController
    #[inline(always)]
    pub fn fan_mode(&self) -> FanControlFanMode {
        let signal = self.raw.view_bits::<Lsb0>()[0..8].load_le::<u8>();
        match signal {
            0 => FanControlFanMode::Off,
            1 => FanControlFanMode::On,
            2 => FanControlFanMode::Auto,
            _ => FanControlFanMode::_Other(self.fan_mode_phys_val()),
        }
    }
    #[inline(always)]
    fn fan_mode_phys_val(&self) -> u8 {
        self.fan_mode_raw_val()
    }
    /// Returns the raw value of `FanMode`.
    ///
    /// - Start bit: 0
    /// - Signal size: 8 bits
    /// - Byte order: LittleEndian
    /// - Value type: Unsigned
    #[inline(always)]
    pub fn fan_mode_raw_val(&self) -> u8 {
        self.raw.view_bits::<Lsb0>()[0..8].load_le::<u8>()
    }
    /// Sets the raw value of `FanMode`.
    #[inline(always)]
    pub fn set_fan_mode_raw_val(&mut self, value: u8) -> Result<(), CanError> {
        if (i128::from(value)) < 0_i128 || (i128::from(value)) > 255_i128 {
            return Err(CanError::ParameterOutOfRange {
                message_id: FanControl::MESSAGE_ID,
            });
        }
        self.raw.view_bits_mut::<Lsb0>()[0..8].store_le(value);
        Ok(())
    }
    /// Sets the value of `FanMode`.
    #[inline(always)]
    pub fn set_fan_mode(&mut self, value: FanControlFanMode) -> Result<(), CanError> {
        let value = u8::from(value);
        if (i128::from(value)) < 0_i128 || (i128::from(value)) > 255_i128 {
            return Err(CanError::ParameterOutOfRange {
                message_id: FanControl::MESSAGE_ID,
            });
        }
        {
            let physical = value as f64 * 1_f64 + 0_f64;
            if physical < 0_f64 || physical > 2_f64 {
                return Err(CanError::ParameterOutOfRange {
                    message_id: FanControl::MESSAGE_ID,
                });
            }
        }
        self.raw.view_bits_mut::<Lsb0>()[0..8].store_le(value);
        Ok(())
    }
    /// Physical value of original signal "TargetTemperature".
    pub fn target_temperature(&self) -> f64 {
        ((self.target_temperature_raw_val() as f64) * 1_f64 + 0_f64) as f64
    }
    /// Returns the raw value of `TargetTemperature`.
    ///
    /// - Start bit: 8
    /// - Signal size: 8 bits
    /// - Byte order: LittleEndian
    /// - Value type: Unsigned
    #[inline(always)]
    pub fn target_temperature_raw_val(&self) -> u8 {
        self.raw.view_bits::<Lsb0>()[8..16].load_le::<u8>()
    }
    /// Sets the raw value of `TargetTemperature`.
    #[inline(always)]
    pub fn set_target_temperature_raw_val(&mut self, value: u8) -> Result<(), CanError> {
        if (i128::from(value)) < 0_i128 || (i128::from(value)) > 255_i128 {
            return Err(CanError::ParameterOutOfRange {
                message_id: FanControl::MESSAGE_ID,
            });
        }
        self.raw.view_bits_mut::<Lsb0>()[8..16].store_le(value);
        Ok(())
    }
    /// Sets a finite physical value. Failure leaves the payload unchanged.
    pub fn set_target_temperature(&mut self, value: f64) -> Result<(), CanError> {
        self.set_target_temperature_quantized(value).map(|_| ())
    }
    /// Sets a value and returns its actual quantized physical value.
    pub fn set_target_temperature_quantized(
        &mut self,
        value: f64,
    ) -> Result<f64, CanError> {
        if !value.is_finite() {
            return Err(CanError::ParameterOutOfRange {
                message_id: FanControl::MESSAGE_ID,
            });
        }
        if (value as f64) < 0_f64 || (value as f64) > 100_f64 {
            return Err(CanError::ParameterOutOfRange {
                message_id: FanControl::MESSAGE_ID,
            });
        }
        let scaled = (value as f64 - 0_f64) / 1_f64;
        if !scaled.is_finite() {
            return Err(CanError::ParameterOutOfRange {
                message_id: FanControl::MESSAGE_ID,
            });
        }
        if scaled < (0_f64 - 1.0) || scaled > (256_f64 + 1.0) {
            return Err(CanError::ParameterOutOfRange {
                message_id: FanControl::MESSAGE_ID,
            });
        }
        let base = scaled as i128;
        let bits = base
            + if scaled - base as f64 >= 0.5 {
                1
            } else if scaled - base as f64 <= -0.5 {
                -1
            } else {
                0
            };
        if (bits) < 0_i128 || (bits) > 255_i128 {
            return Err(CanError::ParameterOutOfRange {
                message_id: FanControl::MESSAGE_ID,
            });
        }
        let value = bits as u8;
        let actual = ((value as f64) * 1_f64 + 0_f64) as f64;
        if !actual.is_finite() {
            return Err(CanError::ParameterOutOfRange {
                message_id: FanControl::MESSAGE_ID,
            });
        }
        if (actual as f64) < 0_f64 || (actual as f64) > 100_f64 {
            return Err(CanError::ParameterOutOfRange {
                message_id: FanControl::MESSAGE_ID,
            });
        }
        self.raw.view_bits_mut::<Lsb0>()[8..16].store_le(value);
        Ok(actual)
    }
}
impl core::convert::TryFrom<&[u8]> for FanControl {
    type Error = CanError;
    #[inline(always)]
    fn try_from(payload: &[u8]) -> Result<Self, Self::Error> {
        if payload.len() != 8 {
            return Err(CanError::InvalidPayloadSize);
        }
        let mut raw = [0u8; 8];
        raw.copy_from_slice(&payload[..8]);
        Ok(Self { raw })
    }
}
impl embedded_can::Frame for FanControl {
    fn new(id: impl Into<Id>, data: &[u8]) -> Option<Self> {
        if id.into() != Self::MESSAGE_ID { None } else { data.try_into().ok() }
    }
    fn new_remote(_id: impl Into<Id>, _dlc: usize) -> Option<Self> {
        unimplemented!()
    }
    fn is_extended(&self) -> bool {
        match self.id() {
            Id::Standard(_) => false,
            Id::Extended(_) => true,
        }
    }
    fn is_remote_frame(&self) -> bool {
        false
    }
    fn id(&self) -> Id {
        Self::MESSAGE_ID
    }
    fn dlc(&self) -> usize {
        self.raw.len()
    }
    fn data(&self) -> &[u8] {
        &self.raw
    }
}
/// Defined values for FanMode
#[allow(
    clippy::absurd_extreme_comparisons,
    clippy::excessive_precision,
    clippy::manual_range_contains,
    clippy::unnecessary_cast,
    clippy::useless_conversion,
    unused_comparisons,
    unused_variables,
)]
#[derive(Clone, Copy, PartialEq)]
pub enum FanControlFanMode {
    Off,
    On,
    Auto,
    _Other(u8),
}
impl From<FanControlFanMode> for u8 {
    fn from(val: FanControlFanMode) -> u8 {
        match val {
            FanControlFanMode::Off => 0,
            FanControlFanMode::On => 1,
            FanControlFanMode::Auto => 2,
            FanControlFanMode::_Other(x) => x,
        }
    }
}
/// FanStatus
///
/// - Standard ID: 258 (0x102)
/// - Size: 8 bytes
/// - Transmitter: FanController
///
/// Published fan controller state.
#[derive(Clone, Copy)]
pub struct FanStatus {
    raw: [u8; 8],
}
#[allow(
    clippy::absurd_extreme_comparisons,
    clippy::excessive_precision,
    clippy::manual_range_contains,
    clippy::unnecessary_cast,
    clippy::useless_conversion,
    unused_comparisons,
    unused_variables,
)]
impl FanStatus {
    pub const MESSAGE_ID: embedded_can::Id = Id::Standard(
        StandardId::new(0x102).unwrap(),
    );
    pub const MESSAGE_SIZE: usize = 8;
    pub const FAN_SPEED_MIN: f64 = 0_f64;
    pub const FAN_SPEED_MAX: f64 = 6000_f64;
    /// Constructs a new `FanStatus` message from values.
    pub fn new(
        fan_speed: f64,
        fan_active: FanStatusFanActive,
    ) -> Result<Self, CanError> {
        let mut res = Self { raw: [0x00; 8] };
        res.set_fan_speed(fan_speed)?;
        res.set_fan_active(fan_active)?;
        Ok(res)
    }
    /// Returns the raw `FanStatus` message payload.
    pub fn raw(&self) -> &[u8; 8] {
        &self.raw
    }
    /// Physical value of original signal "FanSpeed".
    pub fn fan_speed(&self) -> f64 {
        ((self.fan_speed_raw_val() as f64) * 1_f64 + 0_f64) as f64
    }
    /// Returns the raw value of `FanSpeed`.
    ///
    /// - Start bit: 0
    /// - Signal size: 16 bits
    /// - Byte order: LittleEndian
    /// - Value type: Unsigned
    #[inline(always)]
    pub fn fan_speed_raw_val(&self) -> u16 {
        self.raw.view_bits::<Lsb0>()[0..16].load_le::<u16>()
    }
    /// Sets the raw value of `FanSpeed`.
    #[inline(always)]
    pub fn set_fan_speed_raw_val(&mut self, value: u16) -> Result<(), CanError> {
        if (i128::from(value)) < 0_i128 || (i128::from(value)) > 65535_i128 {
            return Err(CanError::ParameterOutOfRange {
                message_id: FanStatus::MESSAGE_ID,
            });
        }
        self.raw.view_bits_mut::<Lsb0>()[0..16].store_le(value);
        Ok(())
    }
    /// Sets a finite physical value. Failure leaves the payload unchanged.
    pub fn set_fan_speed(&mut self, value: f64) -> Result<(), CanError> {
        self.set_fan_speed_quantized(value).map(|_| ())
    }
    /// Sets a value and returns its actual quantized physical value.
    pub fn set_fan_speed_quantized(&mut self, value: f64) -> Result<f64, CanError> {
        if !value.is_finite() {
            return Err(CanError::ParameterOutOfRange {
                message_id: FanStatus::MESSAGE_ID,
            });
        }
        if (value as f64) < 0_f64 || (value as f64) > 6000_f64 {
            return Err(CanError::ParameterOutOfRange {
                message_id: FanStatus::MESSAGE_ID,
            });
        }
        let scaled = (value as f64 - 0_f64) / 1_f64;
        if !scaled.is_finite() {
            return Err(CanError::ParameterOutOfRange {
                message_id: FanStatus::MESSAGE_ID,
            });
        }
        if scaled < (0_f64 - 1.0) || scaled > (65536_f64 + 1.0) {
            return Err(CanError::ParameterOutOfRange {
                message_id: FanStatus::MESSAGE_ID,
            });
        }
        let base = scaled as i128;
        let bits = base
            + if scaled - base as f64 >= 0.5 {
                1
            } else if scaled - base as f64 <= -0.5 {
                -1
            } else {
                0
            };
        if (bits) < 0_i128 || (bits) > 65535_i128 {
            return Err(CanError::ParameterOutOfRange {
                message_id: FanStatus::MESSAGE_ID,
            });
        }
        let value = bits as u16;
        let actual = ((value as f64) * 1_f64 + 0_f64) as f64;
        if !actual.is_finite() {
            return Err(CanError::ParameterOutOfRange {
                message_id: FanStatus::MESSAGE_ID,
            });
        }
        if (actual as f64) < 0_f64 || (actual as f64) > 6000_f64 {
            return Err(CanError::ParameterOutOfRange {
                message_id: FanStatus::MESSAGE_ID,
            });
        }
        self.raw.view_bits_mut::<Lsb0>()[0..16].store_le(value);
        Ok(actual)
    }
    /// Returns the value of `FanActive`.
    ///
    /// 1 while FanSpeed is greater than zero.
    ///
    /// - Min: 0
    /// - Max: 1
    /// - Unit: Not specified
    /// - Receivers: ClimateController, HMI
    #[inline(always)]
    pub fn fan_active(&self) -> FanStatusFanActive {
        let signal = self.raw.view_bits::<Lsb0>()[16..17].load_le::<u8>();
        match signal {
            0 => FanStatusFanActive::Off,
            1 => FanStatusFanActive::On,
            _ => FanStatusFanActive::_Other(self.fan_active_phys_val()),
        }
    }
    #[inline(always)]
    fn fan_active_phys_val(&self) -> u8 {
        self.fan_active_raw_val()
    }
    /// Returns the raw value of `FanActive`.
    ///
    /// - Start bit: 16
    /// - Signal size: 1 bits
    /// - Byte order: LittleEndian
    /// - Value type: Unsigned
    #[inline(always)]
    pub fn fan_active_raw_val(&self) -> u8 {
        self.raw.view_bits::<Lsb0>()[16..17].load_le::<u8>()
    }
    /// Sets the raw value of `FanActive`.
    #[inline(always)]
    pub fn set_fan_active_raw_val(&mut self, value: u8) -> Result<(), CanError> {
        if (i128::from(value)) < 0_i128 || (i128::from(value)) > 1_i128 {
            return Err(CanError::ParameterOutOfRange {
                message_id: FanStatus::MESSAGE_ID,
            });
        }
        self.raw.view_bits_mut::<Lsb0>()[16..17].store_le(value);
        Ok(())
    }
    /// Sets the value of `FanActive`.
    #[inline(always)]
    pub fn set_fan_active(&mut self, value: FanStatusFanActive) -> Result<(), CanError> {
        let value = u8::from(value);
        if (i128::from(value)) < 0_i128 || (i128::from(value)) > 1_i128 {
            return Err(CanError::ParameterOutOfRange {
                message_id: FanStatus::MESSAGE_ID,
            });
        }
        {
            let physical = value as f64 * 1_f64 + 0_f64;
            if physical < 0_f64 || physical > 1_f64 {
                return Err(CanError::ParameterOutOfRange {
                    message_id: FanStatus::MESSAGE_ID,
                });
            }
        }
        self.raw.view_bits_mut::<Lsb0>()[16..17].store_le(value);
        Ok(())
    }
}
impl core::convert::TryFrom<&[u8]> for FanStatus {
    type Error = CanError;
    #[inline(always)]
    fn try_from(payload: &[u8]) -> Result<Self, Self::Error> {
        if payload.len() != 8 {
            return Err(CanError::InvalidPayloadSize);
        }
        let mut raw = [0u8; 8];
        raw.copy_from_slice(&payload[..8]);
        Ok(Self { raw })
    }
}
impl embedded_can::Frame for FanStatus {
    fn new(id: impl Into<Id>, data: &[u8]) -> Option<Self> {
        if id.into() != Self::MESSAGE_ID { None } else { data.try_into().ok() }
    }
    fn new_remote(_id: impl Into<Id>, _dlc: usize) -> Option<Self> {
        unimplemented!()
    }
    fn is_extended(&self) -> bool {
        match self.id() {
            Id::Standard(_) => false,
            Id::Extended(_) => true,
        }
    }
    fn is_remote_frame(&self) -> bool {
        false
    }
    fn id(&self) -> Id {
        Self::MESSAGE_ID
    }
    fn dlc(&self) -> usize {
        self.raw.len()
    }
    fn data(&self) -> &[u8] {
        &self.raw
    }
}
/// Defined values for FanActive
#[allow(
    clippy::absurd_extreme_comparisons,
    clippy::excessive_precision,
    clippy::manual_range_contains,
    clippy::unnecessary_cast,
    clippy::useless_conversion,
    unused_comparisons,
    unused_variables,
)]
#[derive(Clone, Copy, PartialEq)]
pub enum FanStatusFanActive {
    Off,
    On,
    _Other(u8),
}
impl From<FanStatusFanActive> for u8 {
    fn from(val: FanStatusFanActive) -> u8 {
        match val {
            FanStatusFanActive::Off => 0,
            FanStatusFanActive::On => 1,
            FanStatusFanActive::_Other(x) => x,
        }
    }
}
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanError {
    UnknownMessageId(embedded_can::Id),
    /// Signal parameter is not within the range
    /// defined in the dbc
    ParameterOutOfRange {
        /// dbc message id
        message_id: embedded_can::Id,
    },
    InvalidPayloadSize,
    /// A conditional signal or nested selector is inactive in this payload.
    InactiveSignal {
        /// DBC message identifier.
        message_id: embedded_can::Id,
        /// Generated signal name.
        signal: &'static str,
    },
    /// Multiplexor value not defined in the dbc
    InvalidMultiplexor {
        /// dbc message id
        message_id: embedded_can::Id,
        /// Multiplexor value not defined in the dbc
        multiplexor: u16,
    },
}
impl core::fmt::Display for CanError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
