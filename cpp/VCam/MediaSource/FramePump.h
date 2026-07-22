// FramePump.h — shared memory IPC for external frame injection.
//
// Rust VirtualCameraWriter creates a file mapping "Global\vcam_frames"
// with layout [Header][NV12 frame w*h*3/2] and writes frames there.
// C++ side (FrameGenerator::Generate, in VCamSource.dll loaded by Frame
// Server) opens the same mapping, reads the latest frame, memcpy into the MF
// sample buffer.
//
// Layout:
//   struct Header { u32 magic; u32 width; u32 height; u32 seq; u32 ready; u32 pad[3]; }
//   u8 frame[width * height * 3 / 2]   // NV12: Y plane + interleaved UV
//
// "Global\" prefix so Frame Server (running as Local Service) can see it
// across sessions. Rust must create with CreateFileMappingW(Global\...,
// security DACL granting Local Service read access).
#pragma once

#include <Windows.h>
#include <cstdint>

class FramePump
{
public:
    #pragma pack(push, 1)
    struct Header {
        uint32_t magic;
        uint32_t width;
        uint32_t height;
        uint32_t seq;       // incremented per frame by the Rust writer
        uint32_t ready;     // 0 = empty, 1 = frame available
        uint32_t pad[3];
    };
    #pragma pack(pop)

    static constexpr DWORD kHeaderSize = sizeof(Header);

    FramePump() : _mapping(nullptr), _view(nullptr), _lastSeq(0) {}

    ~FramePump()
    {
        if (_view) UnmapViewOfFile(_view);
        if (_mapping) CloseHandle(_mapping);
    }

    // Try to open the shared memory mapping created by the Rust sender.
    // Returns S_OK if mapping is open and header looks valid, else error.
    // Safe to call repeatedly — re-opens if mapping was lost.
    HRESULT EnsureConnected()
    {
        if (_view) {
            // Still mapped; verify header is alive.
            auto* hdr = static_cast<Header*>(_view);
            if (hdr->magic == vcam::contract::SharedMemoryMagic) return S_OK;
        }

        if (!_mapping) {
            _mapping = OpenFileMappingW(FILE_MAP_READ, FALSE, vcam::contract::SharedMemoryName);
            if (!_mapping) return HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND);
        }

        if (!_view) {
            _view = MapViewOfFile(_mapping, FILE_MAP_READ, 0, 0, 0);
            if (!_view) {
                CloseHandle(_mapping);
                _mapping = nullptr;
                return HRESULT_FROM_WIN32(GetLastError());
            }
        }

        auto* hdr = static_cast<Header*>(_view);
        if (hdr->magic != vcam::contract::SharedMemoryMagic) {
            // Sender hasn't initialized the header yet.
            return E_NOTIMPL;
        }
        return S_OK;
    }

    // Copy the latest frame into the provided MF sample buffer.
    // `dst` is the locked IMF2DBuffer2 scanline pointer, `dstLen` is its length.
    // Returns S_OK if a fresh frame was copied, S_FALSE if no new frame, else error.
    HRESULT CopyLatestFrame(uint8_t* dst, LONG dstPitch, DWORD dstLen,
        uint32_t expectedWidth, uint32_t expectedHeight)
    {
        auto hr = EnsureConnected();
        if (FAILED(hr)) return hr;

        auto* hdr = static_cast<Header*>(_view);
        if (!hdr->ready) return S_FALSE;            // no frame yet
        // The mapping is FILE_MAP_READ. An Interlocked RMW used as a load would
        // fault on this page, so use an aligned volatile load plus barriers.
        const auto seqBefore = *reinterpret_cast<volatile const uint32_t*>(&hdr->seq);
        MemoryBarrier();
        if (seqBefore & 1) return S_FALSE;           // writer is mid-copy
        if (seqBefore == _lastSeq) return S_FALSE;   // already copied this one
        if (hdr->width != expectedWidth || hdr->height != expectedHeight) {
            return HRESULT_FROM_WIN32(ERROR_INVALID_DATA);
        }

        // NV12 frame size = width * height * 3 / 2
        const DWORD pitch = static_cast<DWORD>(dstPitch < 0 ? -dstPitch : dstPitch);
        const DWORD required = pitch * hdr->height + pitch * (hdr->height / 2);
        if (!dst || pitch < hdr->width || dstLen < required) {
            return HRESULT_FROM_WIN32(ERROR_INSUFFICIENT_BUFFER);
        }

        auto* src = static_cast<uint8_t*>(_view) + kHeaderSize;
        // IMF2DBuffer2 surfaces may pad each row. Copy the Y and UV planes
        // row-by-row instead of assuming a tightly packed destination.
        for (uint32_t y = 0; y < hdr->height; ++y) {
            CopyMemory(dst + y * pitch, src + y * hdr->width, hdr->width);
        }
        auto* srcUV = src + hdr->width * hdr->height;
        auto* dstUV = dst + pitch * hdr->height;
        for (uint32_t y = 0; y < hdr->height / 2; ++y) {
            CopyMemory(dstUV + y * pitch, srcUV + y * hdr->width, hdr->width);
        }
        MemoryBarrier();
        const auto seqAfter = *reinterpret_cast<volatile const uint32_t*>(&hdr->seq);
        if (seqAfter != seqBefore || (seqAfter & 1)) return S_FALSE;
        _lastSeq = seqAfter;
        return S_OK;
    }

    // Accessors for FrameGenerator to query negotiated size.
    uint32_t Width() const  { return _view ? static_cast<Header*>(_view)->width  : 0; }
    uint32_t Height() const { return _view ? static_cast<Header*>(_view)->height : 0; }

private:
    HANDLE _mapping;
    void* _view;
    uint32_t _lastSeq;
};
