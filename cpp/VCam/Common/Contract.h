#pragma once

#include <cstdint>

#ifdef _WIN32
#include <guiddef.h>
#endif

namespace vcam::contract
{
#ifdef _WIN32
    // Stable COM identity. Changing this GUID requires uninstalling the old
    // source registration before installing the new build.
    inline constexpr GUID SourceClsid = {
        0x3cad447d, 0xf283, 0x4af4, { 0xa3, 0xb2, 0x6f, 0x53, 0x63, 0x30, 0x9f, 0x52 }
    };
#endif

    inline constexpr wchar_t FriendlyName[] = L"VCam";
    inline constexpr wchar_t SharedMemoryName[] = L"Global\\vcam_frames";
    inline constexpr wchar_t WriterMutexName[] = L"Global\\vcam_frames_writer";
    inline constexpr wchar_t RegistrarReadyEventName[] = L"Global\\vcam_registrar_ready";
    inline constexpr std::uint32_t SharedMemoryMagic = 0x4d414356; // "VCAM"
    inline constexpr std::uint32_t HeaderVersion = 2;
    inline constexpr std::uint32_t HeaderWords = 12;

    inline constexpr std::uint32_t DefaultWidth = 1280;
    inline constexpr std::uint32_t DefaultHeight = 720;
    inline constexpr std::uint32_t DefaultFpsNumerator = 30;
    inline constexpr std::uint32_t DefaultFpsDenominator = 1;
    inline constexpr std::uint32_t MaxWidth = 4096;
    inline constexpr std::uint32_t MaxHeight = 2160;
    inline constexpr std::uint32_t MaxFrameBytes = MaxWidth * MaxHeight * 3 / 2;

    #pragma pack(push, 1)
    struct SharedHeader {
        std::uint32_t magic;
        std::uint32_t version;
        std::uint32_t headerBytes;
        std::uint32_t width;
        std::uint32_t height;
        std::uint32_t fpsNumerator;
        std::uint32_t fpsDenominator;
        std::uint32_t frameBytes;
        std::uint32_t seq;
        std::uint32_t ready;
        std::uint32_t reserved[2];
    };
    #pragma pack(pop)

    static_assert(sizeof(SharedHeader) == HeaderWords * sizeof(std::uint32_t));

    struct HeaderSnapshot {
        std::uint32_t magic;
        std::uint32_t version;
        std::uint32_t headerBytes;
        std::uint32_t width;
        std::uint32_t height;
        std::uint32_t fpsNumerator;
        std::uint32_t fpsDenominator;
        std::uint32_t frameBytes;
        std::uint32_t seq;
        std::uint32_t ready;
    };

    inline HeaderSnapshot SnapshotHeader(const volatile SharedHeader& header) noexcept
    {
        return {
            header.magic,
            header.version,
            header.headerBytes,
            header.width,
            header.height,
            header.fpsNumerator,
            header.fpsDenominator,
            header.frameBytes,
            header.seq,
            header.ready,
        };
    }

    inline constexpr bool IsStableSequence(
        std::uint32_t seqBefore,
        const HeaderSnapshot& snapshot,
        std::uint32_t seqAfter) noexcept
    {
        return (seqBefore & 1) == 0
            && snapshot.seq == seqBefore
            && seqAfter == seqBefore;
    }

    inline constexpr bool IsValidHeaderSnapshot(const HeaderSnapshot& snapshot) noexcept
    {
        if (snapshot.magic != SharedMemoryMagic
            || snapshot.version != HeaderVersion
            || snapshot.headerBytes != sizeof(SharedHeader)
            || snapshot.width == 0 || snapshot.height == 0
            || (snapshot.width & 1) || (snapshot.height & 1)
            || snapshot.width > MaxWidth || snapshot.height > MaxHeight
            || snapshot.fpsNumerator == 0 || snapshot.fpsDenominator == 0
            || snapshot.ready > 1) {
            return false;
        }

        const auto expectedBytes = static_cast<std::uint64_t>(snapshot.width)
            * snapshot.height * 3 / 2;
        return expectedBytes == snapshot.frameBytes
            && expectedBytes <= MaxFrameBytes;
    }

    inline constexpr bool IsValidForwardNv12Destination(
        std::int32_t pitch,
        std::uint64_t destinationBytes,
        std::uint32_t width,
        std::uint32_t height) noexcept
    {
        if (pitch <= 0 || width == 0 || height == 0) return false;
        const auto pitchBytes = static_cast<std::uint64_t>(pitch);
        const auto rows = static_cast<std::uint64_t>(height) + height / 2;
        return pitchBytes >= width && pitchBytes * rows <= destinationBytes;
    }
}
