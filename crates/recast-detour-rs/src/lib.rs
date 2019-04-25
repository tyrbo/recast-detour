use std::ffi::CStr;
use std::os::raw::c_void;
use std::ptr;

#[derive(Debug)]
pub struct RecastQuery {
    q: ptr::NonNull<c_void>,
}

impl Drop for RecastQuery {
    fn drop(&mut self) {
        unsafe { sys::recastc_free_query(self.q.as_ptr()) }
    }
}

#[derive(Debug)]
pub enum Error {
    CreateQueryError(String),
    FindPointError(String),
    FindPathError(String),
}

type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub struct NavMeshData {
    /// Vertices in world unit, length = 3 * Number of Vertices
    vertices: Vec<f32>,
    /// Indices,  length = 3 * Number of Triangles
    indices: Vec<u16>,
    /// Walkable height in nav mesh in World Unit
    walkable_height: f32,
    /// Walkable Radius in nav mesh in World Unit
    walkable_radius: f32,
    /// Walkable climb height in World Unit
    walkable_climb: f32,

    /// Cell size in world unit
    cell_size: f32,
    /// Cell height in world unit
    cell_height: f32,
}

fn compute_bb(vertices: &[f32]) -> ([f32; 3], [f32; 3]) {
    let mut bmin = [std::f32::MAX; 3];
    let mut bmax = [std::f32::MIN; 3];
    debug_assert!(vertices.len() % 3 == 0);

    for i in (0..vertices.len()).step_by(3) {
        bmin[0] = vertices[i + 0].min(bmin[0]);
        bmin[1] = vertices[i + 1].min(bmin[1]);
        bmin[2] = vertices[i + 2].min(bmin[2]);

        bmax[0] = vertices[i + 0].max(bmax[0]);
        bmax[1] = vertices[i + 1].max(bmax[1]);
        bmax[2] = vertices[i + 2].max(bmax[2]);
    }

    (bmin, bmax)
}

#[inline]
fn world_unit_to_cell_unit(f: f32, bmin: f32, cs: f32) -> u16 {
    let f = ((f - bmin) / cs).max(0.0);
    f as u16
}

#[derive(Debug, Copy, Clone, Default)]
pub struct Point([f32; 3]);

impl Point {
    pub fn new((x, y, z): (f32, f32, f32)) -> Point {
        Point([x, y, z])
    }
}

impl From<(f32, f32, f32)> for Point {
    fn from(f: (f32, f32, f32)) -> Point {
        Point::new(f)
    }
}

impl RecastQuery {
    /// Create a query from NavMesh
    pub fn new_from_mesh(data: NavMeshData) -> Result<RecastQuery> {
        let (bmin, bmax) = compute_bb(&data.vertices);

        let mut cu_verts = Vec::new();

        // World Unit to Cell Unit
        for i in (0..data.vertices.len()).step_by(3) {
            for j in 0..3 {
                cu_verts.push(world_unit_to_cell_unit(
                    data.vertices[i + j],
                    bmin[j],
                    data.cell_size,
                ));
            }
        }

        let vert_count = (data.vertices.len() / 3) as u32;
        let triangles_count = (data.indices.len() / 3) as u32;

        let sys_data = sys::RecastNavMeshData {
            verts: cu_verts.as_ptr(),
            vert_count,
            indices: data.indices.as_ptr(),
            triangles_count,
            bmin,
            bmax,
            walkable_height: data.walkable_height,
            walkable_radius: data.walkable_radius,
            walkable_climb: data.walkable_climb,
            cell_size: data.cell_size,
            cell_height: data.cell_height,
        };

        let mut err = sys::RecastNavError::zeros();

        let q = unsafe {
            ptr::NonNull::new(
                sys::recastc_create_query(&sys_data as *const _, &mut err as *mut _) as *mut c_void,
            )
        };

        let q = q.ok_or(Error::CreateQueryError(err.msg().into_owned()))?;
        Ok(RecastQuery { q })
    }

    pub fn find_path(&self, start: Point, end: Point, r: f32) -> Result<Point> {
        let (start_p, start_poly) = self.find_poly(start, r)?;
        let (end_p, end_poly) = self.find_poly(end, r)?;

        let mut result = sys::RecastPathResult::default();
        let mut err = sys::RecastNavError::zeros();

        let input = sys::RecastPathInput {
            start_poly,
            start_pos: start_p.0,
            end_poly,
            end_pos: end_p.0,
        };

        let res = unsafe {
            sys::recastc_find_path(
                self.q.as_ptr(),
                &input as *const _,
                &mut result as *mut _,
                &mut err as *mut _,
            )
        };

        if res == 0 {
            return Err(Error::FindPathError(err.msg().to_string()));
        }

        let path = &result.path[0..result.path_count as usize];

        match path.len() {
            0 => Err(Error::FindPathError("No Path".to_string())),
            // Same Poly, so just return the next point
            1 => Ok(end_p),
            _ => self.find_closest(start_p, path[1]),
        }
    }

    fn find_closest(&self, pos: Point, target_poly: u32) -> Result<Point> {
        let input = sys::RecastClosestPointInput {
            pos: pos.0,
            poly: target_poly,
        };

        let mut result = sys::RecastClosestPointResult::default();
        let mut err = sys::RecastNavError::zeros();
        let res = unsafe {
            sys::recastc_find_closest_point(
                self.q.as_ptr(),
                &input as *const _,
                &mut result as *mut _,
                &mut err as *mut _,
            )
        };

        if res == 0 {
            Err(Error::FindPointError(err.msg().to_string()))
        } else {
            Ok(Point(result.pos))
        }
    }

    fn find_poly(&self, pos: Point, r: f32) -> Result<(Point, u32)> {
        let mut result = sys::RecastNearestPointResult::default();
        let mut err = sys::RecastNavError::zeros();

        let input = sys::RecastNearestPointInput {
            center: pos.0,
            half_extents: [r, r, r],
        };

        let res = unsafe {
            sys::recastc_find_nearest_point(
                self.q.as_ptr(),
                &input as *const _,
                &mut result as *mut _,
                &mut err as *mut _,
            )
        };

        match res {
            0 => Err(Error::FindPointError(err.msg().to_string())),
            _ if result.poly == 0 => Err(Error::FindPointError("No poly found".into())),
            _ => Ok((Point(result.pos), result.poly)),
        }
    }
}

pub fn version() -> String {
    let version = unsafe { sys::recastc_version() };
    assert_ne!(version, ptr::null());
    let version = unsafe { CStr::from_ptr(version).to_str().unwrap() };
    version.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use insta::*;

    fn simple_mesh() -> NavMeshData {
        let vertices = vec![
            0.0, 0.0, 0.0, 10.0, 0.0, 0.0, 10.0, 0.0, 10.0, 0.0, 0.0, 10.0,
        ];

        let indices = vec![0, 1, 2, 0, 2, 3];

        NavMeshData {
            vertices,
            indices,
            walkable_height: 0.2,
            walkable_radius: 0.2,
            walkable_climb: 0.2,
            cell_size: 0.1,
            cell_height: 0.1,
        }
    }

    #[test]
    fn smoke_test() {
        assert_eq!("0.0.1", version());
        let mesh = simple_mesh();

        let q = RecastQuery::new_from_mesh(mesh).unwrap();
        drop(q);
    }

    #[test]
    fn test_simple_path() {
        assert_eq!("0.0.1", version());
        let mesh = simple_mesh();

        let q = RecastQuery::new_from_mesh(mesh).unwrap();
        let p = q
            .find_path((0.2, 0.1, 0.4).into(), (0.8, 0.1, 0.5).into(), 0.2)
            .unwrap();

        assert_debug_snapshot_matches!(p, @r###"Point(
    [
        0.29999924,
        0.0,
        0.29999924
    ]
)"###);
    }
}