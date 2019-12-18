//! This crate defines a buffer data structure optimized to be written to and read from standard
//! `Vec`s. `DataBuffer` is particularly useful when dealing with data whose type is determined at
//! run time.  Note that data is stored in the underlying byte buffers in native endian form, thus
//! requesting typed data from a buffer on a platform with different endianness is unsafe.
//!
//! # Caveats
//!
//! `DataBuffer` doesn't support zero-sized types.

pub use reinterpret;

use std::{
    any::{Any, TypeId},
    mem::size_of,
    slice,
};

#[cfg(feature = "numeric")]
use std::fmt;

#[cfg(feature = "numeric")]
use num_traits::{cast, NumCast, Zero};

pub mod macros;

#[cfg(feature = "serde")]
mod serde_helpers {
    use std::any::TypeId;
    fn transmute_type_id_to_u64(id: &TypeId) -> u64 {
        unsafe { std::mem::transmute::<TypeId, u64>(*id) }
    }

    #[derive(serde::Serialize, serde::Deserialize)]
    #[serde(remote = "TypeId")]
    pub struct TypeIdDef {
        #[serde(getter = "transmute_type_id_to_u64")]
        t: u64,
    }

    impl From<TypeIdDef> for TypeId {
        fn from(def: TypeIdDef) -> TypeId {
            unsafe { std::mem::transmute::<u64, TypeId>(def.t) }
        }
    }
}

/// Buffer of data. The data is stored as an array of bytes (`Vec<u8>`).
/// `DataBuffer` keeps track of the type stored within via an explicit `TypeId` member. This allows
/// one to hide the type from the compiler and check it only when necessary. It is particularly
/// useful when the type of data is determined at runtime (e.g. when parsing numeric data).
#[derive(Clone, Debug, PartialEq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DataBuffer {
    /// Raw data stored as bytes.
    #[cfg_attr(feature = "serde_bytes", serde(with = "serde_bytes"))]
    data: Vec<usize>,
    /// Number of bytes occupied by an element of this buffer.
    ///
    /// Note: We store this instead of length because it gives us the ability to get the type size
    /// when the buffer is empty.
    element_size: usize,
    /// Type encoding for hiding the type of data from the compiler.
    #[cfg_attr(feature = "serde", serde(with = "serde_helpers::TypeIdDef"))]
    element_type_id: TypeId,
}

impl DataBuffer {
    /// Construct an empty `DataBuffer` with a specific type.
    #[inline]
    pub fn with_type<T: Any>() -> Self {
        let element_size = size_of::<T>();
        assert_ne!(
            element_size, 0,
            "DataBuffer doesn't support zero sized types."
        );
        DataBuffer {
            data: Vec::new(),
            element_size,
            element_type_id: TypeId::of::<T>(),
        }
    }

    /// Construct a `DataBuffer` with the same type as the given buffer without copying its data.
    #[inline]
    pub fn with_buffer_type(other: &DataBuffer) -> Self {
        DataBuffer {
            data: Vec::new(),
            element_size: other.element_size,
            element_type_id: other.element_type_id,
        }
    }

    /// Construct an empty `DataBuffer` with a capacity for a given number of typed elements. For
    /// setting byte capacity use `with_byte_capacity`.
    #[inline]
    pub fn with_capacity<T: Any>(n: usize) -> Self {
        let element_size = size_of::<T>();
        assert_ne!(
            element_size, 0,
            "DataBuffer doesn't support zero sized types."
        );
        DataBuffer {
            data: Vec::with_capacity(n * element_size),
            element_size,
            element_type_id: TypeId::of::<T>(),
        }
    }

    /// Construct a typed `DataBuffer` with a given size and filled with the specified default
    /// value.
    /// #  Examples
    /// ```
    /// # extern crate data_buffer as buf;
    /// # use buf::DataBuffer;
    /// # fn main() {
    /// let buf = DataBuffer::with_size(8, 42usize); // Create buffer
    /// let buf_vec: Vec<usize> = buf.into_vec().unwrap(); // Convert into `Vec`
    /// assert_eq!(buf_vec, vec![42usize; 8]);
    /// # }
    /// ```
    #[inline]
    pub fn with_size<T: Any + Clone>(n: usize, def: T) -> Self {
        Self::from_vec(vec![def; n])
    }

    /// Construct a `DataBuffer` from a given `Vec<T>` reusing the space already allocated by the
    /// given vector.
    /// #  Examples
    /// ```
    /// # extern crate data_buffer as buf;
    /// # use buf::DataBuffer;
    /// # fn main() {
    /// let vec = vec![1u8, 3, 4, 1, 2];
    /// let buf = DataBuffer::from_vec(vec.clone()); // Convert into buffer
    /// let nu_vec: Vec<u8> = buf.into_vec().unwrap(); // Convert back into `Vec`
    /// assert_eq!(vec, nu_vec);
    /// # }
    /// ```
    pub fn from_vec<T: Any>(mut vec: Vec<T>) -> Self {
        let element_size = size_of::<T>();
        assert_ne!(
            element_size, 0,
            "DataBuffer doesn't support zero sized types."
        );

        let data = {
            let len_in_bytes = vec.len() * element_size;
            let capacity_in_bytes = vec.capacity() * element_size;
            let vec_ptr = vec.as_mut_ptr() as *mut u8;

            unsafe {
                ::std::mem::forget(vec);
                Vec::from_raw_parts(vec_ptr, len_in_bytes, capacity_in_bytes)
            }
        };

        DataBuffer {
            data,
            element_size,
            element_type_id: TypeId::of::<T>(),
        }
    }

    /// Construct a `DataBuffer` from a given slice by cloning the data.
    #[inline]
    pub fn from_slice<T: Any + Clone>(slice: &[T]) -> Self {
        let mut vec = Vec::with_capacity(slice.len());
        vec.extend_from_slice(slice);
        Self::from_vec(vec)
    }

    /// Resizes the buffer in-place to store `new_len` elements and returns an optional
    /// mutable reference to `Self`.
    ///
    /// If `T` does not correspond to the underlying element type, then `None` is returned and the
    /// `DataBuffer` is left unchanged.
    ///
    /// This function has the similar properties to `Vec::resize`.
    #[inline]
    pub fn resize<T: Any + Clone>(&mut self, new_len: usize, value: T) -> Option<&mut Self> {
        self.check_ref::<T>()?;
        let size_t = size_of::<T>();
        if new_len >= self.len() {
            let diff = new_len - self.len();
            self.reserve_bytes(diff * size_t);
            for _ in 0..diff {
                self.push(value.clone());
            }
        } else {
            // Truncate
            self.data.resize(new_len * size_t, 0);
        }
        Some(self)
    }

    /// Copy data from a given slice into the current buffer.
    ///
    /// The `DataBuffer` is extended if the given slice is larger than the number of elements
    /// already stored in this `DataBuffer`.
    #[inline]
    pub fn copy_from_slice<T: Any + Copy>(&mut self, slice: &[T]) -> &mut Self {
        let element_size = size_of::<T>();
        assert_ne!(
            element_size, 0,
            "DataBuffer doesn't support zero sized types."
        );
        let bins = slice.len() * element_size;
        let byte_slice = unsafe { slice::from_raw_parts(slice.as_ptr() as *const u8, bins) };
        self.data.resize(bins, 0);
        self.data.copy_from_slice(byte_slice);
        self.element_size = element_size;
        self.element_type_id = TypeId::of::<T>();
        self
    }

    /// Clear the data buffer without destroying its type information.
    #[inline]
    pub fn clear(&mut self) {
        self.data.clear();
    }

    /// Fill the current buffer with copies of the given value. The size of the buffer is left
    /// unchanged. If the given type doesn't patch the internal type, `None` is returned, otherwise
    /// a mut reference to the modified buffer is returned.
    /// #  Examples
    /// ```
    /// # extern crate data_buffer as buf;
    /// # use buf::DataBuffer;
    /// # fn main() {
    /// let vec = vec![1u8, 3, 4, 1, 2];
    /// let mut buf = DataBuffer::from_vec(vec.clone()); // Convert into buffer
    /// buf.fill(0u8);
    /// assert_eq!(buf.into_vec::<u8>().unwrap(), vec![0u8, 0, 0, 0, 0]);
    /// # }
    /// ```
    #[inline]
    pub fn fill<T: Any + Clone>(&mut self, def: T) -> Option<&mut Self> {
        for v in self.iter_mut::<T>()? {
            *v = def.clone();
        }
        Some(self)
    }

    /// Add an element to this buffer. If the type of the given element coincides with the type
    /// stored by this buffer, then the modified buffer is returned via a mutable reference.
    /// Otherwise, `None` is returned.
    #[inline]
    pub fn push<T: Any>(&mut self, element: T) -> Option<&mut Self> {
        self.check_ref::<T>()?;
        let element_ref = &element;
        let element_byte_ptr = element_ref as *const T as *const u8;
        let element_byte_slice = unsafe { slice::from_raw_parts(element_byte_ptr, size_of::<T>()) };
        unsafe { self.push_bytes(element_byte_slice) }
    }

    /// Check if the current buffer contains elements of the specified type. Returns `Some(self)`
    /// if the type matches and `None` otherwise.
    #[inline]
    pub fn check<T: Any>(self) -> Option<Self> {
        if TypeId::of::<T>() != self.element_type_id() {
            None
        } else {
            Some(self)
        }
    }

    /// Check if the current buffer contains elements of the specified type. Returns `None` if the
    /// check fails, otherwise a reference to self is returned.
    #[inline]
    pub fn check_ref<T: Any>(&self) -> Option<&Self> {
        if TypeId::of::<T>() != self.element_type_id() {
            None
        } else {
            Some(self)
        }
    }

    /// Check if the current buffer contains elements of the specified type. Same as `check_ref`
    /// but consumes and produces a mut reference to self.
    #[inline]
    pub fn check_mut<'a, T: Any>(&'a mut self) -> Option<&'a mut Self> {
        if TypeId::of::<T>() != self.element_type_id() {
            None
        } else {
            Some(self)
        }
    }

    /*
     * Accessors
     */

    /// Get the `TypeId` of data stored within this buffer.
    #[inline]
    pub fn element_type_id(&self) -> TypeId {
        self.element_type_id
    }

    /// Get the number of elements stored in this buffer.
    #[inline]
    pub fn len(&self) -> usize {
        debug_assert_eq!(self.data.len() % self.element_size, 0);
        self.data.len() / self.element_size // element_size is guaranteed to be strictly positive
    }

    /// Check if there are any elements stored in this buffer.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Get the byte capacity of this buffer.
    #[inline]
    pub fn byte_capacity(&self) -> usize {
        self.data.capacity()
    }

    /// Get the size of the element type in bytes.
    #[inline]
    pub fn element_size(&self) -> usize {
        self.element_size
    }

    /// Return an iterator to a slice representing typed data.
    /// Returs `None` if the given type `T` doesn't match the internal.
    /// # Examples
    /// ```
    /// # extern crate data_buffer as buf;
    /// # use buf::DataBuffer;
    /// # fn main() {
    /// let vec = vec![1.0_f32, 23.0, 0.01, 42.0, 11.43];
    /// let buf = DataBuffer::from(vec.clone()); // Convert into buffer
    /// for (i, &val) in buf.iter::<f32>().unwrap().enumerate() {
    ///     assert_eq!(val, vec[i]);
    /// }
    /// # }
    /// ```
    #[inline]
    pub fn iter<'a, T: Any + 'a>(&'a self) -> Option<slice::Iter<T>> {
        self.as_slice::<T>().map(|x| x.iter())
    }

    /// Return an iterator to a mutable slice representing typed data.
    /// Returs `None` if the given type `T` doesn't match the internal.
    #[inline]
    pub fn iter_mut<'a, T: Any + 'a>(&'a mut self) -> Option<slice::IterMut<T>> {
        self.as_mut_slice::<T>().map(|x| x.iter_mut())
    }

    /// Append cloned items from this buffer to a given `Vec<T>`. Return the mutable reference
    /// `Some(vec)` if type matched the internal type and `None` otherwise.
    #[inline]
    pub fn append_clone_to_vec<'a, T>(&self, vec: &'a mut Vec<T>) -> Option<&'a mut Vec<T>>
    where
        T: Any + Clone,
    {
        vec.extend_from_slice(self.as_slice()?);
        Some(vec)
    }

    /// Append copied items from this buffer to a given `Vec<T>`. Return the mutable reference
    /// `Some(vec)` if type matched the internal type and `None` otherwise. This may be faster than
    /// `append_clone_to_vec`.
    #[inline]
    pub fn append_copy_to_vec<'a, T>(&self, vec: &'a mut Vec<T>) -> Option<&'a mut Vec<T>>
    where
        T: Any + Copy,
    {
        vec.extend(self.iter()?);
        Some(vec)
    }

    /// Clones contents of `self` into the given `Vec`.
    #[inline]
    pub fn clone_into_vec<T: Any + Clone>(&self) -> Option<Vec<T>> {
        let mut vec = Vec::<T>::with_capacity(self.len());
        match self.append_clone_to_vec(&mut vec) {
            Some(_) => Some(vec),
            None => None,
        }
    }

    /// Copies contents of `self` into the given `Vec`.
    #[inline]
    pub fn copy_into_vec<T: Any + Copy>(&self) -> Option<Vec<T>> {
        let mut vec = Vec::<T>::with_capacity(self.len());
        match self.append_copy_to_vec(&mut vec) {
            Some(_) => Some(vec),
            None => None,
        }
    }

    /// An alternative to using the `Into` trait. This function helps the compiler
    /// determine the type `T` automatically.
    #[inline]
    pub fn into_vec<T: Any>(self) -> Option<Vec<T>> {
        unsafe { self.check::<T>().map(|x| x.reinterpret_into_vec()) }
    }

    /// Convert this buffer into a typed slice.
    /// Returs `None` if the given type `T` doesn't match the internal.
    #[inline]
    pub fn as_slice<T: Any>(&self) -> Option<&[T]> {
        let ptr = self.check_ref::<T>()?.data.as_ptr() as *const T;
        Some(unsafe { slice::from_raw_parts(ptr, self.len()) })
    }

    /// Convert this buffer into a typed mutable slice.
    /// Returs `None` if the given type `T` doesn't match the internal.
    #[inline]
    pub fn as_mut_slice<T: Any>(&mut self) -> Option<&mut [T]> {
        let ptr = self.check_mut::<T>()?.data.as_mut_ptr() as *mut T;
        Some(unsafe { slice::from_raw_parts_mut(ptr, self.len()) })
    }

    /// Get `i`'th element of the buffer by value.
    #[inline]
    pub fn get<T: Any + Copy>(&self, i: usize) -> Option<T> {
        assert!(i < self.len());
        let ptr = self.check_ref::<T>()?.data.as_ptr() as *const T;
        Some(unsafe { *ptr.add(i) })
    }

    /// Get a `const` reference to the `i`'th element of the buffer.
    #[inline]
    pub fn get_ref<T: Any>(&self, i: usize) -> Option<&T> {
        assert!(i < self.len());
        let ptr = self.check_ref::<T>()?.data.as_ptr() as *const T;
        Some(unsafe { &*ptr.add(i) })
    }

    /// Get a mutable reference to the `i`'th element of the buffer.
    #[inline]
    pub fn get_mut<T: Any>(&mut self, i: usize) -> Option<&mut T> {
        assert!(i < self.len());
        let ptr = self.check_mut::<T>()?.data.as_mut_ptr() as *mut T;
        Some(unsafe { &mut *ptr.add(i) })
    }

    /*
     * Advanced methods to probe buffer internals.
     */

    /// Reserves capacity for at least `additional` more bytes to be inserted in this buffer.
    #[inline]
    pub fn reserve_bytes(&mut self, additional: usize) {
        self.data.reserve(additional);
    }

    /// Get `i`'th element of the buffer by value without checking type.
    /// This can be used to reinterpret the internal data as a different type. Note that if the
    /// size of the given type `T` doesn't match the size of the internal type, `i` will really
    /// index the `i`th `T` sized chunk in the current buffer. See the implementation for details.
    #[inline]
    pub unsafe fn get_unchecked<T: Any + Copy>(&self, i: usize) -> T {
        let ptr = self.data.as_ptr() as *const T;
        *ptr.add(i)
    }

    /// Get a `const` reference to the `i`'th element of the buffer.
    /// This can be used to reinterpret the internal data as a different type. Note that if the
    /// size of the given type `T` doesn't match the size of the internal type, `i` will really
    /// index the `i`th `T` sized chunk in the current buffer. See the implementation for details.
    #[inline]
    pub unsafe fn get_unchecked_ref<T: Any>(&self, i: usize) -> &T {
        let ptr = self.data.as_ptr() as *const T;
        &*ptr.add(i)
    }

    /// Get a mutable reference to the `i`'th element of the buffer.
    /// This can be used to reinterpret the internal data as a different type. Note that if the
    /// size of the given type `T` doesn't match the size of the internal type, `i` will really
    /// index the `i`th `T` sized chunk in the current buffer. See the implementation for details.
    #[inline]
    pub unsafe fn get_unchecked_mut<T: Any>(&mut self, i: usize) -> &mut T {
        let ptr = self.data.as_mut_ptr() as *mut T;
        &mut *ptr.add(i)
    }

    /// Get a `const` reference to the byte slice of the `i`'th element of the buffer.
    #[inline]
    pub fn get_bytes(&self, i: usize) -> &[u8] {
        debug_assert!(i < self.len());
        let element_size = self.element_size();
        &self.data[i * element_size..(i + 1) * element_size]
    }

    /// Get a mutable reference to the byte slice of the `i`'th element of the buffer.
    ///
    /// # Unsafety
    ///
    /// This function is marked as unsafe since the returned bytes may be modified
    /// arbitrarily, which may potentially produce malformed values.
    #[inline]
    pub unsafe fn get_bytes_mut(&mut self, i: usize) -> &mut [u8] {
        debug_assert!(i < self.len());
        let element_size = self.element_size();
        &mut self.data[i * element_size..(i + 1) * element_size]
    }

    /// Move buffer data to a vector with a given type, reinterpreting the data type as
    /// required.
    #[inline]
    pub unsafe fn reinterpret_into_vec<T>(self) -> Vec<T> {
        reinterpret::reinterpret_vec(self.data)
    }

    /// Borrow buffer data and reinterpret it as a slice of a given type.
    #[inline]
    pub unsafe fn reinterpret_as_slice<T>(&self) -> &[T] {
        reinterpret::reinterpret_slice(self.data.as_slice())
    }

    /// Mutably borrow buffer data and reinterpret it as a mutable slice of a given type.
    #[inline]
    pub unsafe fn reinterpret_as_mut_slice<T>(&mut self) -> &mut [T] {
        reinterpret::reinterpret_mut_slice(self.data.as_mut_slice())
    }

    /// Borrow buffer data and iterate over reinterpreted underlying data.
    #[inline]
    pub unsafe fn reinterpret_iter<T>(&self) -> slice::Iter<T> {
        self.reinterpret_as_slice().iter()
    }

    /// Mutably borrow buffer data and mutably iterate over reinterpreted underlying data.
    #[inline]
    pub unsafe fn reinterpret_iter_mut<T>(&mut self) -> slice::IterMut<T> {
        self.reinterpret_as_mut_slice().iter_mut()
    }

    /// Peak at the internal representation of the data.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        self.data.as_slice()
    }

    /// Get a mutable reference to the internal data representation.
    ///
    /// # Unsafety
    ///
    /// This function is marked as unsafe since the returned bytes may be modified
    /// arbitrarily, which may potentially produce malformed values.
    #[inline]
    pub unsafe fn as_bytes_mut(&mut self) -> &mut [u8] {
        self.data.as_mut_slice()
    }

    /// Iterate over chunks type sized chunks of bytes without interpreting them. This avoids
    /// needing to know what type data you're dealing with. This type of iterator is useful for
    /// transferring data from one place to another for a generic buffer.
    #[inline]
    pub fn byte_chunks<'a>(&'a self) -> impl Iterator<Item = &'a [u8]> + 'a {
        let chunk_size = self.element_size();
        self.data.chunks(chunk_size)
    }

    /// Mutably iterate over chunks type sized chunks of bytes without interpreting them. This
    /// avoids needing to know what type data you're dealing with. This type of iterator is useful
    /// for transferring data from one place to another for a generic buffer, or modifying the
    /// underlying untyped bytes (e.g. bit twiddling).
    ///
    /// # Unsafety
    ///
    /// This function is marked as unsafe since the returned bytes may be modified
    /// arbitrarily, which may potentially produce malformed values.
    #[inline]
    pub unsafe fn byte_chunks_mut<'a>(&'a mut self) -> impl Iterator<Item = &'a mut [u8]> + 'a {
        let chunk_size = self.element_size();
        self.data.chunks_mut(chunk_size)
    }

    /// Add bytes to this buffer. If the size of the given slice coincides with the number of bytes
    /// occupied by the underlying element type, then these bytes are added to the underlying data
    /// buffer and a mutable reference to the buffer is returned.
    /// Otherwise, `None` is returned, and the buffer remains unmodified.
    #[inline]
    pub unsafe fn push_bytes(&mut self, bytes: &[u8]) -> Option<&mut Self> {
        if bytes.len() == self.element_size() {
            self.data.extend_from_slice(bytes);
            Some(self)
        } else {
            None
        }
    }

    /// Add bytes to this buffer. If the size of the given slice is a multiple of the number of bytes
    /// occupied by the underlying element type, then these bytes are added to the underlying data
    /// buffer and a mutable reference to the buffer is returned.
    /// Otherwise, `None` is returned and the buffer is unmodified.
    #[inline]
    pub unsafe fn extend_bytes(&mut self, bytes: &[u8]) -> Option<&mut Self> {
        let element_size = self.element_size();
        if bytes.len() % element_size == 0 {
            self.data.extend_from_slice(bytes);
            Some(self)
        } else {
            None
        }
    }

    /// Move bytes to this buffer. If the size of the given vector is a multiple of the number of bytes
    /// occupied by the underlying element type, then these bytes are moved to the underlying data
    /// buffer and a mutable reference to the buffer is returned.
    /// Otherwise, `None` is returned and both the buffer and the input vector remain unmodified.
    #[inline]
    pub unsafe fn append_bytes(&mut self, bytes: &mut Vec<u8>) -> Option<&mut Self> {
        let element_size = self.element_size();
        if bytes.len() % element_size == 0 {
            self.data.append(bytes);
            Some(self)
        } else {
            None
        }
    }

    /// Move bytes to this buffer. The given buffer must have the same underlying type as self.
    #[inline]
    pub fn append(&mut self, buf: &mut DataBuffer) -> Option<&mut Self> {
        if buf.element_type_id() == self.element_type_id() {
            self.data.append(&mut buf.data);
            Some(self)
        } else {
            None
        }
    }

    /// Rotates the slice in-place such that the first `mid` elements of the slice move to the end
    /// while the last `self.len() - mid` elements move to the front. After calling `rotate_left`,
    /// the element previously at index `mid` will become the first element in the slice.
    ///
    /// # Example
    ///
    /// ```
    /// # use data_buffer::*;
    /// let mut buf = DataBuffer::from_vec(vec![1u32,2,3,4,5]);
    /// buf.rotate_left(3);
    /// assert_eq!(buf.as_slice::<u32>().unwrap(), &[4,5,1,2,3]);
    /// ```
    #[inline]
    pub fn rotate_left(&mut self, mid: usize) {
        self.data.rotate_left(mid * self.element_size);
    }

    /// Rotates the slice in-place such that the first `self.len() - k` elements of the slice move
    /// to the end while the last `k` elements move to the front. After calling `rotate_right`, the
    /// element previously at index `k` will become the first element in the slice.
    ///
    /// # Example
    ///
    /// ```
    /// # use data_buffer::*;
    /// let mut buf = DataBuffer::from_vec(vec![1u32,2,3,4,5]);
    /// buf.rotate_right(3);
    /// assert_eq!(buf.as_slice::<u32>().unwrap(), &[3,4,5,1,2]);
    /// ```
    #[inline]
    pub fn rotate_right(&mut self, k: usize) {
        self.data.rotate_right(k * self.element_size);
    }

    /*
     * Methods specific to buffers storing numeric data
     */

    #[cfg(feature = "numeric")]
    /// Cast a numeric `DataBuffer` into the given output `Vec` type.
    pub fn cast_into_vec<T>(self) -> Vec<T>
    where
        T: Any + Copy + NumCast + Zero,
    {
        // Helper function (generic on the input) to convert the given DataBuffer into Vec.
        unsafe fn convert_into_vec<I, O>(buf: DataBuffer) -> Vec<O>
        where
            I: Any + NumCast,
            O: Any + Copy + NumCast + Zero,
        {
            debug_assert_eq!(buf.element_type_id(), TypeId::of::<I>()); // Check invariant.
            buf.reinterpret_into_vec()
                .into_iter()
                .map(|elem: I| cast(elem).unwrap_or(O::zero()))
                .collect()
        }
        call_numeric_buffer_fn!( convert_into_vec::<_,T>(self) or { Vec::new() } )
    }

    #[cfg(feature = "numeric")]
    /// Display the contents of this buffer reinterpreted in the given type.
    unsafe fn reinterpret_display<T: Any + fmt::Display>(&self, f: &mut fmt::Formatter) {
        debug_assert_eq!(self.element_type_id(), TypeId::of::<T>()); // Check invariant.
        for item in self.reinterpret_iter::<T>() {
            write!(f, "{} ", item).expect("Error occurred while writing an DataBuffer.");
        }
    }
}

/// Convert a `Vec<T>` to a `DataBuffer`.
impl<T> From<Vec<T>> for DataBuffer
where
    T: Any,
{
    #[inline]
    fn from(vec: Vec<T>) -> DataBuffer {
        DataBuffer::from_vec(vec)
    }
}

/// Convert a `&[T]` to a `DataBuffer`.
impl<'a, T> From<&'a [T]> for DataBuffer
where
    T: Any + Clone,
{
    #[inline]
    fn from(slice: &'a [T]) -> DataBuffer {
        DataBuffer::from_slice(slice)
    }
}

/// Convert a `DataBuffer` to a `Option<Vec<T>>`.
impl<T> Into<Option<Vec<T>>> for DataBuffer
where
    T: Any + Clone,
{
    #[inline]
    fn into(self) -> Option<Vec<T>> {
        self.into_vec()
    }
}

#[cfg(feature = "numeric")]
/// Implement pretty printing of numeric `DataBuffer` data.
impl fmt::Display for DataBuffer {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        call_numeric_buffer_fn!( self.reinterpret_display::<_>(f) or {
            println!("Unknown DataBuffer type for pretty printing.");
        } );
        write!(f, "")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test various ways to create a data buffer.
    #[test]
    fn initialization_test() {
        // Empty typed buffer.
        let a = DataBuffer::with_type::<f32>();
        assert_eq!(a.len(), 0);
        assert_eq!(a.as_bytes().len(), 0);
        assert_eq!(a.element_type_id(), TypeId::of::<f32>());
        assert_eq!(a.byte_capacity(), 0); // Ensure nothing is allocated.

        // Empty buffer typed by the given type id.
        let b = DataBuffer::with_buffer_type(&a);
        assert_eq!(b.len(), 0);
        assert_eq!(b.as_bytes().len(), 0);
        assert_eq!(b.element_type_id(), TypeId::of::<f32>());
        assert_eq!(a.byte_capacity(), 0); // Ensure nothing is allocated.

        // Empty typed buffer with a given capacity.
        let a = DataBuffer::with_capacity::<f32>(4);
        assert_eq!(a.len(), 0);
        assert_eq!(a.as_bytes().len(), 0);
        assert_eq!(a.byte_capacity(), 4 * size_of::<f32>());
        assert_eq!(a.element_type_id(), TypeId::of::<f32>());
    }

    /// Test reserving capacity after creation.
    #[test]
    fn reserve_bytes() {
        let mut a = DataBuffer::with_type::<f32>();
        assert_eq!(a.byte_capacity(), 0);
        a.reserve_bytes(10);
        assert_eq!(a.len(), 0);
        assert_eq!(a.as_bytes().len(), 0);
        assert!(a.byte_capacity() >= 10);
    }

    /// Test resizing a buffer.
    #[test]
    fn resize() {
        let mut a = DataBuffer::with_type::<f32>();

        // Increase the size of a.
        a.resize(3, 1.0f32);

        assert_eq!(a.len(), 3);
        assert_eq!(a.as_bytes().len(), 12);
        for i in 0..3 {
            assert_eq!(a.get::<f32>(i).unwrap(), 1.0f32);
        }

        // Truncate a.
        a.resize(2, 1.0f32);

        assert_eq!(a.len(), 2);
        assert_eq!(a.as_bytes().len(), 8);
        for i in 0..2 {
            assert_eq!(a.get::<f32>(i).unwrap(), 1.0f32);
        }
    }

    #[test]
    #[should_panic]
    fn zero_size_with_type_test() {
        let _a = DataBuffer::with_type::<()>();
    }

    #[test]
    #[should_panic]
    fn zero_size_with_capacity_test() {
        let _a = DataBuffer::with_capacity::<()>(2);
    }

    #[test]
    #[should_panic]
    fn zero_size_from_vec_test() {
        let _a = DataBuffer::from_vec(vec![(); 3]);
    }

    #[test]
    #[should_panic]
    fn zero_size_with_size_test() {
        let _a = DataBuffer::with_size(3, ());
    }

    #[test]
    #[should_panic]
    fn zero_size_from_slice_test() {
        let v = vec![(); 3];
        let _a = DataBuffer::from_slice(&v);
    }

    #[test]
    #[should_panic]
    fn zero_size_copy_from_slice_test() {
        let v = vec![(); 3];
        let mut a = DataBuffer::with_size(0, 1i32);
        a.copy_from_slice(&v);
    }

    #[test]
    fn data_integrity_u8_test() {
        let vec = vec![1u8, 3, 4, 1, 2];
        let buf = DataBuffer::from(vec.clone()); // Convert into buffer
        let nu_vec: Vec<u8> = buf.copy_into_vec().unwrap(); // Convert back into vec
        assert_eq!(vec, nu_vec);

        let vec = vec![1u8, 3, 4, 1, 2, 52, 1, 3, 41, 23, 2];
        let buf = DataBuffer::from(vec.clone()); // Convert into buffer
        let nu_vec: Vec<u8> = buf.copy_into_vec().unwrap(); // Convert back into vec
        assert_eq!(vec, nu_vec);
    }

    #[test]
    fn data_integrity_i16_test() {
        let vec = vec![1i16, -3, 1002, -231, 32];
        let buf = DataBuffer::from(vec.clone()); // Convert into buffer
        let nu_vec: Vec<i16> = buf.copy_into_vec().unwrap(); // Convert back into vec
        assert_eq!(vec, nu_vec);

        let vec = vec![1i16, -3, 1002, -231, 32, 42, -123, 4];
        let buf = DataBuffer::from(vec.clone()); // Convert into buffer
        let nu_vec: Vec<i16> = buf.copy_into_vec().unwrap(); // Convert back into vec
        assert_eq!(vec, nu_vec);
    }

    #[test]
    fn data_integrity_i32_test() {
        let vec = vec![1i32, -3, 1002, -231, 32];
        let buf = DataBuffer::from(vec.clone()); // Convert into buffer
        let nu_vec: Vec<i32> = buf.into_vec().unwrap(); // Convert back into vec
        assert_eq!(vec, nu_vec);

        let vec = vec![1i32, -3, 1002, -231, 32, 42, -123];
        let buf = DataBuffer::from(vec.clone()); // Convert into buffer
        let nu_vec: Vec<i32> = buf.into_vec().unwrap(); // Convert back into vec
        assert_eq!(vec, nu_vec);
    }

    #[test]
    fn data_integrity_f32_test() {
        let vec = vec![1.0_f32, 23.0, 0.01, 42.0, 11.43];
        let buf = DataBuffer::from(vec.clone()); // Convert into buffer
        let nu_vec: Vec<f32> = buf.into_vec().unwrap(); // Convert back into vec
        assert_eq!(vec, nu_vec);

        let vec = vec![1.0_f32, 23.0, 0.01, 42.0, 11.43, 2e-1];
        let buf = DataBuffer::from(vec.clone()); // Convert into buffer
        let nu_vec: Vec<f32> = buf.into_vec().unwrap(); // Convert back into vec
        assert_eq!(vec, nu_vec);
    }

    #[test]
    fn data_integrity_f64_test() {
        let vec = vec![1f64, -3.0, 10.02, -23.1, 32e-1];
        let buf = DataBuffer::from(vec.clone()); // Convert into buffer
        let nu_vec: Vec<f64> = buf.copy_into_vec().unwrap(); // Convert back into vec
        assert_eq!(vec, nu_vec);

        let vec = vec![1f64, -3.1, 100.2, -2.31, 3.2, 4e2, -1e23];
        let buf = DataBuffer::from(vec.clone()); // Convert into buffer
        let nu_vec: Vec<f64> = buf.copy_into_vec().unwrap(); // Convert back into vec
        assert_eq!(vec, nu_vec);
    }

    #[cfg(feature = "numeric")]
    #[test]
    fn convert_float_test() {
        let vecf64 = vec![1f64, -3.0, 10.02, -23.1, 32e-1];
        let buf = DataBuffer::from(vecf64.clone()); // Convert into buffer
        let nu_vec: Vec<f32> = buf.cast_into_vec(); // Convert back into vec
        let vecf32 = vec![1f32, -3.0, 10.02, -23.1, 32e-1];
        assert_eq!(vecf32, nu_vec);

        let buf = DataBuffer::from(vecf32.clone()); // Convert into buffer
        let nu_vec: Vec<f64> = buf.cast_into_vec(); // Convert back into vec
        for (&a, &b) in vecf64.iter().zip(nu_vec.iter()) {
            assert!((a - b).abs() < 1e-6f64 * f64::max(a, b).abs());
        }

        let vecf64 = vec![1f64, -3.1, 100.2, -2.31, 3.2, 4e2, -1e23];
        let buf = DataBuffer::from(vecf64.clone()); // Convert into buffer
        let nu_vec: Vec<f32> = buf.cast_into_vec(); // Convert back into vec
        let vecf32 = vec![1f32, -3.1, 100.2, -2.31, 3.2, 4e2, -1e23];
        assert_eq!(vecf32, nu_vec);
        let buf = DataBuffer::from(vecf32.clone()); // Convert into buffer
        let nu_vec: Vec<f64> = buf.cast_into_vec(); // Convert back into vec
        for (&a, &b) in vecf64.iter().zip(nu_vec.iter()) {
            assert!((a - b).abs() < 1e-6 * f64::max(a, b).abs());
        }
    }

    #[derive(Clone, Debug, PartialEq)]
    struct Foo {
        a: u8,
        b: i64,
        c: f32,
    }

    #[test]
    fn from_empty_vec_test() {
        let vec: Vec<u32> = Vec::new();
        let buf = DataBuffer::from(vec.clone()); // Convert into buffer
        let nu_vec: Vec<u32> = buf.into_vec().unwrap(); // Convert back into vec
        assert_eq!(vec, nu_vec);

        let vec: Vec<String> = Vec::new();
        let buf = DataBuffer::from(vec.clone()); // Convert into buffer
        let nu_vec: Vec<String> = buf.into_vec().unwrap(); // Convert back into vec
        assert_eq!(vec, nu_vec);

        let vec: Vec<Foo> = Vec::new();
        let buf = DataBuffer::from(vec.clone()); // Convert into buffer
        let nu_vec: Vec<Foo> = buf.into_vec().unwrap(); // Convert back into vec
        assert_eq!(vec, nu_vec);
    }

    #[test]
    fn from_struct_test() {
        let f1 = Foo {
            a: 3,
            b: -32,
            c: 54.2,
        };
        let f2 = Foo {
            a: 33,
            b: -3342432412,
            c: 323454.2,
        };
        let vec = vec![f1.clone(), f2.clone()];
        let buf = DataBuffer::from(vec.clone()); // Convert into buffer
        assert_eq!(f1, buf.get_ref::<Foo>(0).unwrap().clone());
        assert_eq!(f2, buf.get_ref::<Foo>(1).unwrap().clone());
        let nu_vec: Vec<Foo> = buf.into_vec().unwrap(); // Convert back into vec
        assert_eq!(vec, nu_vec);
    }

    #[test]
    fn from_strings_test() {
        let vec = vec![
            String::from("hi"),
            String::from("hello"),
            String::from("goodbye"),
            String::from("bye"),
            String::from("supercalifragilisticexpialidocious"),
            String::from("42"),
        ];
        let buf = DataBuffer::from(vec.clone()); // Convert into buffer
        assert_eq!("hi", buf.get_ref::<String>(0).unwrap());
        assert_eq!("hello", buf.get_ref::<String>(1).unwrap());
        assert_eq!("goodbye", buf.get_ref::<String>(2).unwrap());
        let nu_vec: Vec<String> = buf.into_vec().unwrap(); // Convert back into vec
        assert_eq!(vec, nu_vec);
    }

    #[test]
    fn iter_test() {
        // Check iterating over data with a larger size than 8 bits.
        let vec_f32 = vec![1.0_f32, 23.0, 0.01, 42.0, 11.43];
        let buf = DataBuffer::from(vec_f32.clone()); // Convert into buffer
        for (i, &val) in buf.iter::<f32>().unwrap().enumerate() {
            assert_eq!(val, vec_f32[i]);
        }

        // Check iterating over data with the same size.
        let vec_u8 = vec![1u8, 3, 4, 1, 2, 4, 128, 32];
        let buf = DataBuffer::from(vec_u8.clone()); // Convert into buffer
        for (i, &val) in buf.iter::<u8>().unwrap().enumerate() {
            assert_eq!(val, vec_u8[i]);
        }

        // Check unsafe functions:
        unsafe {
            // TODO: feature gate these two tests for little endian platforms.
            // Check iterating over data with a larger size than input.
            let vec_u32 = vec![17_040_129u32, 545_260_546]; // little endian
            let buf = DataBuffer::from(vec_u8.clone()); // Convert into buffer
            for (i, &val) in buf.reinterpret_iter::<u32>().enumerate() {
                assert_eq!(val, vec_u32[i]);
            }

            // Check iterating over data with a smaller size than input
            let mut buf2 = DataBuffer::from(vec_u32); // Convert into buffer
            for (i, &val) in buf2.reinterpret_iter::<u8>().enumerate() {
                assert_eq!(val, vec_u8[i]);
            }

            // Check mut iterator
            buf2.reinterpret_iter_mut::<u8>().for_each(|val| *val += 1);

            let u8_check_vec = vec![2u8, 4, 5, 2, 3, 5, 129, 33];
            assert_eq!(buf2.reinterpret_into_vec::<u8>(), u8_check_vec);
        }
    }

    #[test]
    fn large_sizes_test() {
        for i in 1000000..1000010 {
            let vec = vec![32u8; i];
            let buf = DataBuffer::from(vec.clone()); // Convert into buffer
            let nu_vec: Vec<u8> = buf.into_vec().unwrap(); // Convert back into vec
            assert_eq!(vec, nu_vec);
        }
    }

    /// This test checks that an error is returned whenever the user tries to access data with the
    /// wrong type data.
    #[test]
    fn wrong_type_test() {
        let vec = vec![1.0_f32, 23.0, 0.01, 42.0, 11.43];
        let mut buf = DataBuffer::from(vec.clone()); // Convert into buffer
        assert_eq!(vec, buf.clone_into_vec::<f32>().unwrap());

        assert!(buf.copy_into_vec::<f64>().is_none());
        assert!(buf.as_slice::<f64>().is_none());
        assert!(buf.as_mut_slice::<u8>().is_none());
        assert!(buf.iter::<[f32; 3]>().is_none());
        assert!(buf.get::<i32>(0).is_none());
        assert!(buf.get_ref::<i32>(1).is_none());
        assert!(buf.get_mut::<i32>(2).is_none());
    }

    /// Test iterating over chunks of data without having to interpret them.
    #[test]
    fn byte_chunks_test() {
        let vec_f32 = vec![1.0_f32, 23.0, 0.01, 42.0, 11.43];
        let buf = DataBuffer::from(vec_f32.clone()); // Convert into buffer

        for (i, val) in buf.byte_chunks().enumerate() {
            assert_eq!(
                unsafe { reinterpret::reinterpret_slice::<u8, f32>(val)[0] },
                vec_f32[i]
            );
        }
    }

    /// Test pushing values and bytes to a buffer.
    #[test]
    fn push_test() {
        let mut vec_f32 = vec![1.0_f32, 23.0, 0.01];
        let mut buf = DataBuffer::from(vec_f32.clone()); // Convert into buffer
        for (i, &val) in buf.iter::<f32>().unwrap().enumerate() {
            assert_eq!(val, vec_f32[i]);
        }

        vec_f32.push(42.0f32);
        buf.push(42.0f32).unwrap(); // must provide explicit type

        for (i, &val) in buf.iter::<f32>().unwrap().enumerate() {
            assert_eq!(val, vec_f32[i]);
        }

        vec_f32.push(11.43);
        buf.push(11.43f32).unwrap();

        for (i, &val) in buf.iter::<f32>().unwrap().enumerate() {
            assert_eq!(val, vec_f32[i]);
        }

        // Zero float is always represented by four zero bytes in IEEE format.
        vec_f32.push(0.0);
        vec_f32.push(0.0);
        unsafe { buf.extend_bytes(&[0, 0, 0, 0, 0, 0, 0, 0]) }.unwrap();

        for (i, &val) in buf.iter::<f32>().unwrap().enumerate() {
            assert_eq!(val, vec_f32[i]);
        }

        // Test byte getters
        for i in 5..7 {
            assert_eq!(buf.get_bytes(i), &[0, 0, 0, 0]);
            assert_eq!(unsafe { buf.get_bytes_mut(i) }, &[0, 0, 0, 0]);
        }

        vec_f32.push(0.0);
        unsafe { buf.push_bytes(&[0, 0, 0, 0]) }.unwrap();

        for (i, &val) in buf.iter::<f32>().unwrap().enumerate() {
            assert_eq!(val, vec_f32[i]);
        }
    }

    /// Test appending to a data buffer from another data buffer.
    #[test]
    fn append_test() {
        let mut buf = DataBuffer::with_type::<f32>(); // Create an empty buffer.

        let data = vec![1.0_f32, 23.0, 0.01, 42.0, 11.43];
        // Append an ordianry vector of data.
        let mut other_buf = DataBuffer::from_vec(data.clone());
        buf.append(&mut other_buf);

        assert!(other_buf.is_empty());

        for (i, &val) in buf.iter::<f32>().unwrap().enumerate() {
            assert_eq!(val, data[i]);
        }
    }

    /// Test appending to a data buffer from other slices and vectors.
    #[test]
    fn extend_append_bytes_test() {
        let mut buf = DataBuffer::with_type::<f32>(); // Create an empty buffer.

        // Append an ordianry vector of data.
        let vec_f32 = vec![1.0_f32, 23.0, 0.01, 42.0, 11.43];
        let mut vec_bytes: Vec<u8> = unsafe { reinterpret::reinterpret_vec(vec_f32.clone()) };
        unsafe { buf.append_bytes(&mut vec_bytes) };

        for (i, &val) in buf.iter::<f32>().unwrap().enumerate() {
            assert_eq!(val, vec_f32[i]);
        }

        buf.clear();
        assert_eq!(buf.len(), 0);

        // Append a temporary vec.
        unsafe { buf.append_bytes(&mut vec![0u8; 4]) };
        assert_eq!(buf.get::<f32>(0).unwrap(), 0.0f32);

        buf.clear();
        assert_eq!(buf.len(), 0);

        // Extend buffer with a slice
        let slice_bytes: &[u8] = unsafe { reinterpret::reinterpret_slice(&vec_f32) };
        unsafe { buf.extend_bytes(slice_bytes) };

        for (i, &val) in buf.iter::<f32>().unwrap().enumerate() {
            assert_eq!(val, vec_f32[i]);
        }
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_test() {
        let vec_f32 = vec![1.0_f32, 23.0, 0.01, 42.0, 11.43];
        let buf = DataBuffer::from(vec_f32.clone()); // Convert into buffer
        dbg!(&buf);
        let buf_str = serde_json::to_string(&buf).expect("Failed to serialize DataBuffer.");
        dbg!(&buf_str);
        let new_buf = serde_json::from_str(&buf_str).expect("Failed to deserialize DataBuffer.");
        dbg!(&new_buf);
        assert_eq!(buf, new_buf);
    }
}
