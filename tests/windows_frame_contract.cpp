#include "../cpp/VCam/Common/Contract.h"

#include <cassert>

int main()
{
    vcam::contract::SharedHeader shared{};
    shared.magic = vcam::contract::SharedMemoryMagic;
    shared.version = vcam::contract::HeaderVersion;
    shared.headerBytes = sizeof(shared);
    shared.width = 1920;
    shared.height = 1080;
    shared.fpsNumerator = 60;
    shared.fpsDenominator = 1;
    shared.frameBytes = 1920 * 1080 * 3 / 2;
    shared.seq = 42;
    shared.ready = 1;

    const auto snapshot = vcam::contract::SnapshotHeader(shared);
    assert(vcam::contract::IsValidHeaderSnapshot(snapshot));
    assert(vcam::contract::IsStableSequence(42, snapshot, 42));

    shared.width = vcam::contract::MaxWidth;
    shared.height = vcam::contract::MaxHeight;
    shared.frameBytes = vcam::contract::MaxFrameBytes;
    assert(snapshot.width == 1920);
    assert(snapshot.height == 1080);
    assert(snapshot.frameBytes == 1920 * 1080 * 3 / 2);

    auto torn = snapshot;
    torn.seq = 44;
    assert(!vcam::contract::IsStableSequence(42, torn, 44));

    auto oversized = snapshot;
    oversized.width = vcam::contract::MaxWidth + 2;
    oversized.frameBytes = UINT32_MAX;
    assert(!vcam::contract::IsValidHeaderSnapshot(oversized));

    const auto destinationBytes = 1920ULL * (1080 + 1080 / 2);
    assert(vcam::contract::IsValidForwardNv12Destination(
        1920, destinationBytes, 1920, 1080));
    assert(!vcam::contract::IsValidForwardNv12Destination(
        -1920, destinationBytes, 1920, 1080));
}
