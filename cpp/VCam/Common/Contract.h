#pragma once

#include <guiddef.h>
#include <cstdint>

namespace vcam::contract
{
    // Stable COM identity. Changing this GUID requires uninstalling the old
    // source registration before installing the new build.
    inline constexpr GUID SourceClsid = {
        0x3cad447d, 0xf283, 0x4af4, { 0xa3, 0xb2, 0x6f, 0x53, 0x63, 0x30, 0x9f, 0x52 }
    };

    inline constexpr wchar_t FriendlyName[] = L"VCam";
    inline constexpr wchar_t SharedMemoryName[] = L"Global\\vcam_frames";
    inline constexpr std::uint32_t SharedMemoryMagic = 0x4d414356; // "VCAM"

    inline constexpr std::uint32_t Width = 1280;
    inline constexpr std::uint32_t Height = 960;
    inline constexpr std::uint32_t Fps = 30;
    inline constexpr std::uint64_t SampleDuration100ns = 10'000'000 / Fps;
}
