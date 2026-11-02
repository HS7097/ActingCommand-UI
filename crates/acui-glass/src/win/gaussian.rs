// SPDX-License-Identifier: GPL-3.0-only
//! Direct2D's Gaussian blur as a Composition effect. Composition takes an
//! effect only as an object that describes itself through these interfaces, and
//! Windows has no such object for this one outside its Win2D library, so this
//! is one. The strength is fixed when it is made.

use std::cell::RefCell;

use windows::core::{implement, Interface, Result as WinResult, GUID, HSTRING, PCWSTR};
use windows::Foundation::{IPropertyValue, PropertyValue};
use windows::Graphics::Effects::{
    IGraphicsEffect, IGraphicsEffectSource, IGraphicsEffectSource_Impl, IGraphicsEffect_Impl,
};
use windows::Win32::Foundation::{E_INVALIDARG, E_POINTER};
use windows::Win32::System::WinRT::Graphics::Direct2D::{
    IGraphicsEffectD2D1Interop, IGraphicsEffectD2D1Interop_Impl, GRAPHICS_EFFECT_PROPERTY_MAPPING,
    GRAPHICS_EFFECT_PROPERTY_MAPPING_DIRECT,
};

/// CLSID_D2D1GaussianBlur.
const GAUSSIAN_BLUR: GUID = GUID::from_u128(0x1feb6d69_2fe6_4ac9_8c58_1d7f93e7a6a5);

#[implement(IGraphicsEffect, IGraphicsEffectSource, IGraphicsEffectD2D1Interop)]
struct Gaussian {
    name: RefCell<HSTRING>,
    sigma: f32,
    source: IGraphicsEffectSource,
}

impl IGraphicsEffectSource_Impl for Gaussian_Impl {}

impl IGraphicsEffect_Impl for Gaussian_Impl {
    fn Name(&self) -> WinResult<HSTRING> {
        Ok(self.name.borrow().clone())
    }

    fn SetName(&self, name: &HSTRING) -> WinResult<()> {
        *self.name.borrow_mut() = name.clone();
        Ok(())
    }
}

impl IGraphicsEffectD2D1Interop_Impl for Gaussian_Impl {
    fn GetEffectId(&self) -> WinResult<GUID> {
        Ok(GAUSSIAN_BLUR)
    }

    fn GetNamedPropertyMapping(
        &self,
        name: &PCWSTR,
        index: *mut u32,
        mapping: *mut GRAPHICS_EFFECT_PROPERTY_MAPPING,
    ) -> WinResult<()> {
        if name.is_null() || index.is_null() || mapping.is_null() {
            return Err(E_POINTER.into());
        }
        // SAFETY: the caller passes a NUL-terminated name, and the three pointers
        // were checked above; "Deviation" is D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION,
        // property 0, and the two out-pointers are the caller's to be written.
        unsafe {
            if name.to_string().ok().as_deref() != Some("Deviation") {
                return Err(E_INVALIDARG.into());
            }
            index.write(0);
            mapping.write(GRAPHICS_EFFECT_PROPERTY_MAPPING_DIRECT);
        }
        Ok(())
    }

    fn GetPropertyCount(&self) -> WinResult<u32> {
        Ok(3)
    }

    /// The standard deviation, D2D1_GAUSSIANBLUR_OPTIMIZATION_BALANCED and
    /// D2D1_BORDER_MODE_HARD. D2D wants all three.
    fn GetProperty(&self, index: u32) -> WinResult<IPropertyValue> {
        match index {
            0 => PropertyValue::CreateSingle(self.sigma)?.cast(),
            1 | 2 => PropertyValue::CreateUInt32(1)?.cast(),
            _ => Err(E_INVALIDARG.into()),
        }
    }

    fn GetSource(&self, index: u32) -> WinResult<IGraphicsEffectSource> {
        if index == 0 {
            Ok(self.source.clone())
        } else {
            Err(E_INVALIDARG.into())
        }
    }

    fn GetSourceCount(&self) -> WinResult<u32> {
        Ok(1)
    }
}

/// The blur of `source`, `sigma` physical px strong.
pub fn blur(sigma: f32, source: IGraphicsEffectSource) -> IGraphicsEffect {
    Gaussian { name: RefCell::new(HSTRING::from("Blur")), sigma, source }.into()
}
