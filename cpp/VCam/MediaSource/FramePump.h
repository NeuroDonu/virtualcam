// FramePump.h - versioned shared-memory transport for external NV12 frames.
#pragma once

#include <Windows.h>
#include <cstdint>
#include <vector>

class FramePump
{
public:
    using Header = vcam::contract::SharedHeader;
    using HeaderSnapshot = vcam::contract::HeaderSnapshot;

    struct Format {
        uint32_t width;
        uint32_t height;
        uint32_t fpsNumerator;
        uint32_t fpsDenominator;
    };

    static constexpr DWORD kHeaderSize = sizeof(Header);
    static constexpr SIZE_T kMappingSize =
        static_cast<SIZE_T>(kHeaderSize) + vcam::contract::MaxFrameBytes;
    static_assert(kHeaderSize == vcam::contract::HeaderWords * sizeof(uint32_t));

    FramePump() : _mapping(nullptr), _view(nullptr) {}

    ~FramePump()
    {
        if (_view) UnmapViewOfFile(_view);
        if (_mapping) CloseHandle(_mapping);
    }

    HRESULT ReadFormat(Format* format)
    {
        if (!format) return E_POINTER;
        *format = DefaultFormat();

        auto hr = EnsureConnected();
        if (FAILED(hr)) return hr == HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND) ? S_OK : hr;

        HeaderSnapshot snapshot{};
        hr = CaptureValidatedHeader(&snapshot, format);
        return hr == S_FALSE ? S_OK : hr;
    }

    HRESULT CopyLatestFrame(uint8_t* dst, LONG dstPitch, DWORD dstLen,
        uint32_t expectedWidth, uint32_t expectedHeight)
    {
        auto hr = EnsureConnected();
        if (FAILED(hr)) return CopyCachedFrame(dst, dstPitch, dstLen, expectedWidth, expectedHeight);

        HeaderSnapshot observed{};
        Format format{};
        hr = CaptureValidatedHeader(&observed, &format);
        if (hr == S_FALSE) {
            return CopyCachedFrame(dst, dstPitch, dstLen, expectedWidth, expectedHeight);
        }
        if (FAILED(hr)) return hr;
        if (format.width != expectedWidth || format.height != expectedHeight) {
            return HRESULT_FROM_WIN32(ERROR_INVALID_DATA);
        }
        const auto snapshot = observed;
        if (!snapshot.ready) {
            return CopyCachedFrame(dst, dstPitch, dstLen, expectedWidth, expectedHeight);
        }

        size_t pitch = 0;
        RETURN_IF_FAILED(ValidateDestination(
            dst, dstPitch, dstLen, snapshot.width, snapshot.height, &pitch));

        auto* src = static_cast<uint8_t*>(_view) + kHeaderSize;
        for (uint32_t y = 0; y < snapshot.height; ++y) {
            CopyMemory(dst + static_cast<size_t>(y) * pitch,
                src + static_cast<size_t>(y) * snapshot.width, snapshot.width);
        }
        auto* srcUV = src + static_cast<size_t>(snapshot.width) * snapshot.height;
        auto* dstUV = dst + pitch * snapshot.height;
        for (uint32_t y = 0; y < snapshot.height / 2; ++y) {
            CopyMemory(dstUV + static_cast<size_t>(y) * pitch,
                srcUV + static_cast<size_t>(y) * snapshot.width, snapshot.width);
        }

        MemoryBarrier();
        const auto seqAfter = static_cast<volatile const Header*>(_view)->seq;
        if (seqAfter != snapshot.seq || (seqAfter & 1)) {
            return CopyCachedFrame(dst, dstPitch, dstLen, expectedWidth, expectedHeight);
        }

        _cachedFrame.resize(snapshot.frameBytes);
        for (uint32_t y = 0; y < snapshot.height; ++y) {
            CopyMemory(_cachedFrame.data() + static_cast<size_t>(y) * snapshot.width,
                dst + static_cast<size_t>(y) * pitch, snapshot.width);
        }
        auto* cachedUV = _cachedFrame.data()
            + static_cast<size_t>(snapshot.width) * snapshot.height;
        for (uint32_t y = 0; y < snapshot.height / 2; ++y) {
            CopyMemory(cachedUV + static_cast<size_t>(y) * snapshot.width,
                dstUV + static_cast<size_t>(y) * pitch, snapshot.width);
        }
        _hasCachedFrame = true;
        return S_OK;
    }

private:
    static constexpr Format DefaultFormat()
    {
        return {
            vcam::contract::DefaultWidth,
            vcam::contract::DefaultHeight,
            vcam::contract::DefaultFpsNumerator,
            vcam::contract::DefaultFpsDenominator,
        };
    }

    HRESULT EnsureConnected()
    {
        if (_view) return S_OK;
        if (!_mapping) {
            _mapping = OpenFileMappingW(FILE_MAP_READ, FALSE, vcam::contract::SharedMemoryName);
            if (!_mapping) return HRESULT_FROM_WIN32(GetLastError());
        }
        _view = MapViewOfFile(_mapping, FILE_MAP_READ, 0, 0, kMappingSize);
        if (!_view) {
            const auto error = GetLastError();
            CloseHandle(_mapping);
            _mapping = nullptr;
            return HRESULT_FROM_WIN32(error);
        }
        return S_OK;
    }

    HRESULT CaptureValidatedHeader(HeaderSnapshot* snapshot, Format* format) const
    {
        if (!_view || !snapshot || !format) return E_POINTER;

        const auto* header = static_cast<volatile const Header*>(_view);
        const auto seqBefore = header->seq;
        MemoryBarrier();
        const auto observed = vcam::contract::SnapshotHeader(*header);
        MemoryBarrier();
        const auto seqAfter = header->seq;

        if (!vcam::contract::IsStableSequence(seqBefore, observed, seqAfter)) {
            return S_FALSE;
        }
        if (observed.magic == 0) return S_FALSE;
        if (!vcam::contract::IsValidHeaderSnapshot(observed)) {
            return HRESULT_FROM_WIN32(ERROR_INVALID_DATA);
        }

        *snapshot = observed;
        *format = {
            observed.width,
            observed.height,
            observed.fpsNumerator,
            observed.fpsDenominator,
        };
        return S_OK;
    }

    static HRESULT ValidateDestination(uint8_t* dst, LONG dstPitch, DWORD dstLen,
        uint32_t width, uint32_t height, size_t* pitch)
    {
        if (!pitch) return E_POINTER;
        if (!dst || !vcam::contract::IsValidForwardNv12Destination(
            dstPitch, dstLen, width, height)) {
            return HRESULT_FROM_WIN32(ERROR_INSUFFICIENT_BUFFER);
        }
        *pitch = static_cast<size_t>(dstPitch);
        return S_OK;
    }

    HRESULT CopyCachedFrame(uint8_t* dst, LONG dstPitch, DWORD dstLen,
        uint32_t width, uint32_t height) const
    {
        size_t pitch = 0;
        RETURN_IF_FAILED(ValidateDestination(dst, dstPitch, dstLen, width, height, &pitch));
        const size_t frameSize = static_cast<size_t>(width) * height * 3 / 2;
        if (!_hasCachedFrame || _cachedFrame.size() != frameSize) return S_FALSE;
        for (uint32_t y = 0; y < height; ++y) {
            CopyMemory(dst + static_cast<size_t>(y) * pitch,
                _cachedFrame.data() + static_cast<size_t>(y) * width, width);
        }
        auto* dstUV = dst + static_cast<size_t>(pitch) * height;
        auto* cachedUV = _cachedFrame.data() + static_cast<size_t>(width) * height;
        for (uint32_t y = 0; y < height / 2; ++y) {
            CopyMemory(dstUV + static_cast<size_t>(y) * pitch,
                cachedUV + static_cast<size_t>(y) * width, width);
        }
        return S_OK;
    }

    HANDLE _mapping;
    void* _view;
    std::vector<uint8_t> _cachedFrame;
    bool _hasCachedFrame = false;
};
