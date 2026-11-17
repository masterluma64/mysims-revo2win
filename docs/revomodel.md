# RevoModel

RevoModel (`RMDL`) is the model format of the Wii releases of MySims, MySims Kingdom and
MySims Agents. The Cozy Bundle releases of MySims and Kingdom on PC replaced it with
WindowsModel (`WMDL`), which holds the same data in a different shape.

| | |
|---|---|
| Type id | `0xF9E50586` in MySims and Kingdom, `0x2954E734` in Agents |
| Found in | `.package` files (DBPF; big-endian `FPBD` in Agents) |
| Magic | `RMDL` |
| Byte order | Big-endian |
| Code | [src/rmdl.rs](../src/rmdl.rs) |

A file is a header, a table of meshes and the meshes, followed by the model's extra parameters
and rigs. Everything is found through absolute offsets. Each mesh is a GX display list that
draws from separate attribute arrays, much as the Wii's GPU takes it.

MorcuTool's `RevoModel.cs` worked out the header, the mesh info and the display lists. The
skinning tables, the extra parameters, the rigs, the size of each element and the layout rules
below were found by comparing the Wii files against the matching Cozy Bundle files: Kingdom
keeps the same resource keys on both, and MySims keeps the same groups.

## Header (0x58 bytes)

| Offset | Type | Name | Notes |
|---|---|---|---|
| 0x00 | char[4] | magic | `RMDL` |
| 0x04 | u32 | version | `0x00000101` in every file |
| 0x08 | vec3 | bounds min | `FLT_MAX` × 3 in every file (unused; the meshes carry real bounds) |
| 0x14 | vec3 | bounds max | `-FLT_MAX` × 3 in every file |
| 0x20 | u32 | mesh count | |
| 0x24 | u32 | mesh table offset | Always 0x58 |
| 0x28 | u32 | extra parameter count | |
| 0x2C | u32 | extra keys offset | `u32[count]` |
| 0x30 | u32 | extra values offset | One NUL-terminated string per key |
| 0x34 | u32 | rig table offset | 0 when there are no rigs |
| 0x38 | u8[32] | reserved | Zero |

The mesh table is `u32[mesh count]` offsets to each mesh's info.

Extra parameters are the same as WindowsModel's: key `0x9281771A` names a shadow model
(`no_shadow`, `round1x1_shadow`, ...) and `0xDFD4F97F` is seen with the value `1`. WindowsModel
stores the values as one block with its size; RevoModel stores only where it starts.

## Mesh info (0x5C bytes)

| Offset | Type | Name | Notes |
|---|---|---|---|
| 0x00 | u32 | display list size | Padded to 32 bytes, often further (see below) |
| 0x04 | u32 | display list offset | 32-byte aligned |
| 0x08 | u32 | array count | |
| 0x0C | u32 | array table offset | |
| 0x10 | u64, u32, u32 | material | Instance, type, group of a Material or MaterialSet |
| 0x20 | vec3 | bounds min | |
| 0x2C | vec3 | bounds max | |
| 0x38 | u32[2] | unknown | The same two words as WindowsModel's "unknown 0" |
| 0x40 | u32 | name | FNV32; usually `fnv32("none")` = `0x2CA33BDB` |
| 0x44 | u8[12] | reserved | Zero |
| 0x50 | u32 | skin offset | 0 when not skinned |
| 0x54 | u32 | bone list offset | 0 when not skinned |
| 0x58 | u32 | rig offset | The mesh's rig (one of the rig table's entries); 0 when not skinned |

Material type ids: `0x01D0E75D` (Material) and `0x02019972` (MaterialSet) in MySims and Kingdom,
`0xE6640542` and `0x787E842A` in Agents. MySims names its materials with FNV64 on the Wii and
FNV32 in the Cozy Bundle, so the instances differ between the two releases.

## Array table

`array count` entries of 8 bytes, sorted by attribute in almost every file:

| Offset | Type | Name | Notes |
|---|---|---|---|
| 0x00 | u8 | attribute | GX attribute: 9 position, 10 normal, 11 colour 0, 12 colour 1, 13–20 texture coordinates 0–7 |
| 0x01 | u8 | element size | 12 for positions and normals, 8 for texture coordinates, 4 for colours |
| 0x02 | u8[2] | padding | Zero |
| 0x04 | u32 | data offset | 32-byte aligned |

All values are floats (`f32` × 3 or × 2), except colours, which are RGBA8. The arrays carry no
length: each holds as many elements as the display list indexes. (MorcuTool read the second
byte as a GX component type and size; it is the element's size in bytes.)

Normals are absent from many Agents meshes. Texture coordinate 0 is the diffuse map's, 1 the
lightmap's, and 2 is used by Kingdom's water and grid shaders, matching WindowsModel's UV sets.

## Display list

A standard GX display list, run with every attribute indexed by a u16 and, in skinned meshes,
a position matrix index (u8) first. Commands seen:

| Byte | Command | Followed by |
|---|---|---|
| `0x00` | NOP | Nothing. The exporter puts one after each run of matrix loads, and pads with them |
| `0x20` | Load position matrix (indexed) | u16 palette index, u16 descriptor `0xB000 + slot × 12` |
| `0x28` | Load normal matrix (indexed) | u16 palette index, u16 descriptor `0x8400 + slot × 9` |
| `0x90` | Triangles | u16 vertex count, vertices (Agents only) |
| `0x98` | Triangle strip | u16 vertex count, vertices |

Each vertex is `[u8 matrix]` then one u16 per array, in array table order. The matrix byte is
`slot × 3` (GX's `PNMTXIDX`), so a batch of triangles can use ten palette entries at most;
the exporter loads them with `0x20`/`0x28` pairs before the strips that use them.

In a strip, triangle *i* is (v*i*, v*i*+1, v*i*+2) when *i* is even and (v*i*+1, v*i*, v*i*+2)
when it is odd. Read that way, every triangle has the same winding as in WindowsModel.

The display list size is padded to a multiple of 32, but the exporter often reserved more:
MySims and Kingdom mostly use powers of two that carry over from one mesh to the next. The
extra space is zeros.

## Skin

At the skin offset:

| Offset | Type | Name | Notes |
|---|---|---|---|
| 0x00 | u32 | palette offset | Directly after this header |
| 0x04 | u32 | scratch offset | Directly after the palette |
| 0x08 | u32 | palette count | |

The palette is `palette count` entries of `f32 weights[4]` and `u8 bones[4]` (20 bytes); an
unused bone is `0xFF` with a weight of 0. Each entry is one blended matrix the display list can
load: all vertices with the same bones and weights share an entry. The bones index the mesh's
bone list. The scratch area is `palette count × 48` bytes, three `(1, 0, 0, 0)` rows per entry
in every file; it looks like room for the game to blend the 3 × 4 matrices into.

The bone list (at the bone list offset) is `u32 count` and the FNV32 names of the bones the
palette indexes. Its order is neither the rig's nor the palette's, and it sometimes lists bones
no entry uses. WindowsModel indexes the rig directly instead. This tool writes the bones the
mesh uses, in rig order.

## Rigs

The rig table is a list of u32 offsets ending with 0. Each rig is:

| Offset | Type | Name | Notes |
|---|---|---|---|
| 0x00 | u32 | bone count | |
| 0x04 | u32 | bone hashes offset | Directly after this header |
| 0x08 | u32 | matrices offset | Directly after the hashes |

followed by `u32 hashes[count]` (FNV32 bone names) and `f32 inverse_bind[count][16]`, in the
same element order as WindowsModel (translation in elements 12–14). A Wii rig may list a few
more bones than the PC one, and in another order; the games match bones by hash.

## Layout

The exporter's order, which a re-written file follows to come out identical:

1. The header, then the mesh table.
2. For each mesh, straight after the previous one: the mesh info; zeros to the next 32-byte
   boundary and 32 more; the display list; the array table; each array on a 32-byte boundary;
   then, when skinned, the skin header, palette, scratch area and bone list, with no padding.
3. The extra keys and values.
4. The rig table, aligned to 4 bytes, and the rigs.
5. In Kingdom, zeros to a multiple of 32 bytes. Agents files end where the last section does.
   MySims files are inconsistent: some are padded to 32, some to 4, some not at all.

## Unknowns

- The two words at mesh info 0x38 (they match WindowsModel, but their meaning is unknown).
- What decides the display list buffer sizes, and the order of a mesh's bone list.
- Whether the games accept primitives other than strips and triangle lists, or other element
  formats; none appear in the shipped files.
