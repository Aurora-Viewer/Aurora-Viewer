# Aurora 3D

`aurora-3d.glb` is the detailed model supplied by the user for material
previews: 2,342 triangles, with its original extruded geometry and UVs.
It is self-contained, including its PNG texture.

The scene, nodes and meshes are named **Aurora 3D**. The material, textures,
images, buffers, buffer views, accessors and samplers share that prefix;
none of the previous model names remain in their metadata.

The preview applies the inspected material to every face. The embedded
material and colour texture remain available when importing the model.

# Spinning cat

`spinning-cat.glb` is "Oiiaioooooiai Cat" by Zhuier
(https://sketchfab.com/3d-models/oiiaioooooiai-cat-30d27bf7fb224849b76e208a6eccdb36),
under CC BY 4.0 (https://creativecommons.org/licenses/by/4.0/): 15,774
triangles, one morph target (the cat flattened into a loaf) and its spin
animation.

Changes from the author's file: the 4096 x 4096 PNG colour texture is
reduced to 1024 x 1024 and stored as a JPEG, and the `.gltf`, its `.bin`
and the texture are packed into one GLB. The geometry, the morph target and
the animation are untouched. The author, the source and the licence stay in
the file's `asset.extras`, with a note of these changes.

The viewer reads the mesh, its morph target and its texture
(`crates/aurora-viewer/src/scene/spinning_cat.rs`); it moves the cat itself.
