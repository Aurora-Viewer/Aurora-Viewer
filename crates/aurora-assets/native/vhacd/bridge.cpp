// Thin ownership boundary for the same V-HACD 4.1 API used by Firestorm's
// LLConvexDecompositionVHACD. Geometry and buffers are validated in Rust.
#define ENABLE_VHACD_IMPLEMENTATION 1
#include "VHACD.h"
#include <memory>

using Progress = bool (*)(void*, double);

struct Callback final : VHACD::IVHACD::IUserCallback {
    VHACD::IVHACD* algorithm = nullptr;
    Progress progress = nullptr;
    void* context = nullptr;
    void Update(double overall, double, const char*, const char*) override {
        if (progress && progress(context, overall)) algorithm->Cancel();
    }
};
struct Result {
    VHACD::IVHACD* algorithm = VHACD::CreateVHACD();
    Callback callback;
    ~Result() { if (algorithm) algorithm->Release(); }
};

extern "C" Result* aurora_vhacd_compute(const float* points, uint32_t point_count,
    const uint32_t* triangles, uint32_t triangle_count, uint32_t resolution,
    uint32_t hulls, uint32_t vertices, double tolerance, uint32_t fill,
    Progress progress, void* context) noexcept {
    try {
        auto result = std::make_unique<Result>();
        if (!result->algorithm) return nullptr;
        auto& callback = result->callback;
        callback.algorithm = result->algorithm;
        callback.progress = progress;
        callback.context = context;
        VHACD::IVHACD::Parameters params;
        params.m_resolution = resolution;
        params.m_maxConvexHulls = hulls;
        params.m_maxNumVerticesPerCH = vertices;
        params.m_minimumVolumePercentErrorAllowed = tolerance;
        params.m_fillMode = static_cast<VHACD::FillMode>(fill);
        params.m_callback = &callback;
        // The Rust worker pool processes models concurrently. Avoid nesting
        // an unbounded set of native threads for each model in a linkset.
        params.m_asyncACD = false;
        if (progress(context, 0)) return nullptr;
        if (!result->algorithm->Compute(points, point_count, triangles, triangle_count, params)) return nullptr;
        if (progress(context, 100)) return nullptr;
        return result.release();
    } catch (...) { return nullptr; }
}
extern "C" void aurora_vhacd_free(Result* result) noexcept { delete result; }
extern "C" uint32_t aurora_vhacd_count(const Result* result) noexcept {
    return result->algorithm->GetNConvexHulls();
}
extern "C" bool aurora_vhacd_hull(const Result* result, uint32_t index,
    float* points, uint32_t* point_count, uint32_t* triangles, uint32_t* triangle_count) noexcept {
    try {
        VHACD::IVHACD::ConvexHull hull;
        if (!result->algorithm->GetConvexHull(index, hull)) return false;
        if (points && triangles) {
            if (*point_count < hull.m_points.size() || *triangle_count < hull.m_triangles.size()) return false;
            for (size_t i = 0; i < hull.m_points.size(); ++i) {
                points[i*3] = static_cast<float>(hull.m_points[i].mX);
                points[i*3+1] = static_cast<float>(hull.m_points[i].mY);
                points[i*3+2] = static_cast<float>(hull.m_points[i].mZ);
            }
            for (size_t i = 0; i < hull.m_triangles.size(); ++i) {
                triangles[i*3] = hull.m_triangles[i].mI0;
                triangles[i*3+1] = hull.m_triangles[i].mI1;
                triangles[i*3+2] = hull.m_triangles[i].mI2;
            }
        }
        *point_count = static_cast<uint32_t>(hull.m_points.size());
        *triangle_count = static_cast<uint32_t>(hull.m_triangles.size());
        return true;
    } catch (...) { return false; }
}
