use std::{
	collections::{HashMap, hash_map::Iter},
	rc::Rc,
};

#[derive(Clone, Default, Debug)]
pub struct RcStrMap(HashMap<Rc<str>, Rc<str>>);

impl RcStrMap {
	#[must_use]
	pub fn new() -> Self {
		Self(HashMap::new())
	}

	#[must_use]
	pub const fn from_hashmap(map: HashMap<Rc<str>, Rc<str>>) -> Self {
		Self(map)
	}

	#[must_use]
	pub fn into_inner(self) -> HashMap<Rc<str>, Rc<str>> {
		self.0
	}

	#[must_use]
	pub const fn inner(&self) -> &HashMap<Rc<str>, Rc<str>> {
		&self.0
	}

	pub fn insert<K, V>(&mut self, key: K, value: V) -> Option<Rc<str>>
	where
		K: Into<Rc<str>>,
		V: Into<Rc<str>>,
	{
		self.0.insert(key.into(), value.into())
	}

	#[must_use]
	pub fn get(&self, key: &str) -> Option<&Rc<str>> {
		self.0.get(key)
	}

	pub fn remove(&mut self, key: &str) -> Option<Rc<str>> {
		self.0.remove(key)
	}

	#[must_use]
	pub fn contains_key(&self, key: &str) -> bool {
		self.0.contains_key(key)
	}

	#[must_use]
	pub fn iter(&self) -> Iter<'_, Rc<str>, Rc<str>> {
		self.0.iter()
	}

	#[must_use]
	pub fn len(&self) -> usize {
		self.0.len()
	}

	#[must_use]
	pub fn is_empty(&self) -> bool {
		self.0.is_empty()
	}

	pub fn clear(&mut self) {
		self.0.clear();
	}
}

impl<K, V, const N: usize> From<[(K, V); N]> for RcStrMap
where
	K: Into<Rc<str>>,
	V: Into<Rc<str>>,
{
	fn from(pairs: [(K, V); N]) -> Self {
		let mut ret = Self::new();
		ret.0.reserve(N);
		for (k, v) in pairs {
			ret.0.insert(k.into(), v.into());
		}
		ret
	}
}

impl<K, V> FromIterator<(K, V)> for RcStrMap
where
	K: Into<Rc<str>>,
	V: Into<Rc<str>>,
{
	fn from_iter<T: IntoIterator<Item = (K, V)>>(iter: T) -> Self {
		let mut ret = Self::new();
		ret.extend(iter);
		ret
	}
}

impl<K, V> Extend<(K, V)> for RcStrMap
where
	K: Into<Rc<str>>,
	V: Into<Rc<str>>,
{
	fn extend<T: IntoIterator<Item = (K, V)>>(&mut self, iter: T) {
		for (k, v) in iter {
			self.0.insert(k.into(), v.into());
		}
	}
}

impl<'a> IntoIterator for &'a RcStrMap {
	type Item = (&'a Rc<str>, &'a Rc<str>);
	type IntoIter = Iter<'a, Rc<str>, Rc<str>>;

	fn into_iter(self) -> Self::IntoIter {
		self.0.iter()
	}
}

impl IntoIterator for RcStrMap {
	type Item = (Rc<str>, Rc<str>);
	type IntoIter = std::collections::hash_map::IntoIter<Rc<str>, Rc<str>>;

	fn into_iter(self) -> Self::IntoIter {
		self.0.into_iter()
	}
}
