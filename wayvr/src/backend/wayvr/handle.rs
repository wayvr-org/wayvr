#[macro_export]
macro_rules! gen_id {
    (
		$container_name:ident,
		$instance_name:ident,
		$cell_name:ident,
		$handle_name:ident) => {
        //ThingCell
        #[derive(Debug)]
        pub struct $cell_name {
            pub obj: $instance_name,
            pub generation: u64,
        }

        //ThingVec
        #[derive(Debug)]
        pub struct $container_name {
            // Vec<Option<ThingCell>>
            pub vec: Vec<Option<$cell_name>>,

            cur_generation: u64,
        }

        //ThingHandle
        #[derive(Default, Debug, Clone, Copy, PartialEq, Hash, Eq)]
        pub struct $handle_name {
            idx: u32,
            generation: u64,
        }

        #[allow(dead_code)]
        impl $handle_name {
            pub const fn reset(&mut self) {
                self.generation = 0;
            }

            pub const fn is_set(&self) -> bool {
                self.generation > 0
            }

            pub const fn id(&self) -> u32 {
                self.idx
            }

            pub const fn new(idx: u32, generation: u64) -> Self {
                Self { idx, generation }
            }
        }

        //ThingVec
        impl $container_name {
            pub const fn new() -> Self {
                Self {
                    vec: Vec::new(),
                    cur_generation: 0,
                }
            }

            pub fn iter(&self) -> impl Iterator<Item = ($handle_name, &$instance_name)> {
                self.vec.iter().enumerate().filter_map(|(idx, opt_cell)| {
                    opt_cell.as_ref().map(|cell| {
                        let handle = $container_name::get_handle(&cell, idx);
                        (handle, &cell.obj)
                    })
                })
            }

            #[allow(dead_code)]
            pub fn iter_mut(
                &mut self,
            ) -> impl Iterator<Item = ($handle_name, &mut $instance_name)> {
                self.vec
                    .iter_mut()
                    .enumerate()
                    .filter_map(|(idx, opt_cell)| {
                        opt_cell.as_mut().map(|cell| {
                            let handle = $container_name::get_handle(&cell, idx);
                            (handle, &mut cell.obj)
                        })
                    })
            }

            pub const fn get_handle(cell: &$cell_name, idx: usize) -> $handle_name {
                $handle_name {
                    idx: idx as u32,
                    generation: cell.generation,
                }
            }

            fn find_unused_idx(&mut self) -> Option<u32> {
                for (num, obj) in self.vec.iter().enumerate() {
                    if obj.is_none() {
                        return Some(num as u32);
                    }
                }
                None
            }

            pub fn add(&mut self, obj: $instance_name) -> $handle_name {
                self.cur_generation += 1;
                let generation = self.cur_generation;

                let unused_idx = self.find_unused_idx();

                let idx = if let Some(idx) = unused_idx {
                    idx
                } else {
                    self.vec.len() as u32
                };

                let handle = $handle_name { idx, generation };

                let cell = $cell_name { obj, generation };

                if let Some(idx) = unused_idx {
                    self.vec[idx as usize] = Some(cell);
                } else {
                    self.vec.push(Some(cell))
                }

                handle
            }

            pub fn remove(&mut self, handle: &$handle_name) {
                // Out of bounds, ignore
                if handle.idx as usize >= self.vec.len() {
                    return;
                }

                // Remove only if the generation matches
                if let Some(cell) = &self.vec[handle.idx as usize] {
                    if cell.generation == handle.generation {
                        self.vec[handle.idx as usize] = None;
                    }
                }
            }

            pub fn get(&self, handle: &$handle_name) -> Option<&$instance_name> {
                // Out of bounds, ignore
                if handle.idx as usize >= self.vec.len() {
                    return None;
                }

                if let Some(cell) = &self.vec[handle.idx as usize] {
                    if cell.generation == handle.generation {
                        return Some(&cell.obj);
                    }
                }

                None
            }

            pub fn get_mut(&mut self, handle: &$handle_name) -> Option<&mut $instance_name> {
                // Out of bounds, ignore
                if handle.idx as usize >= self.vec.len() {
                    return None;
                }

                if let Some(cell) = &mut self.vec[handle.idx as usize] {
                    if cell.generation == handle.generation {
                        return Some(&mut cell.obj);
                    }
                }

                None
            }
        }
    };
}

/* Example usage:
gen_id!(ThingVec, ThingInstance, ThingCell, ThingHandle);

struct ThingInstance {}

impl ThingInstance {}
 */
