// Native bounded rate/pitch sink; owner serialization is enforced by the parent registry.
#import <AVFoundation/AVFoundation.h>
#include <math.h>
#include <stdint.h>
#include <string.h>

@interface OrynRateSink : NSObject
@property(nonatomic,strong) AVAudioEngine *engine;
@property(nonatomic,strong) AVAudioPlayerNode *player;
@property(nonatomic,strong) AVAudioUnitTimePitch *stretch;
@property(nonatomic,strong) AVAudioUnitVarispeed *varispeed;
@property(nonatomic,strong) AVAudioFormat *format;
@property(nonatomic,strong) NSLock *lock;
@property(nonatomic) NSUInteger queued;
@property(nonatomic) BOOL retired;
@property(nonatomic,strong) NSData *latestPCM;
@property(nonatomic) double outputRate;
@property(nonatomic) unsigned outputChannels;
@property(nonatomic) uint64_t sequence;
@property(nonatomic) uint64_t dropped;
@end
@implementation OrynRateSink
@end

static BOOL OrynRateConnect(AVAudioEngine *engine,AVAudioNode *source,
    AVAudioNode *destination,AVAudioFormat *format,NSError **error) {
    if (@available(macOS 27.0,*)) {
        return [engine connect:source to:destination format:format error:error];
    }
    // Earlier supported SDK APIs throw on invalid connection. The open path
    // catches those exceptions and never returns a partly configured graph.
    [engine connect:source to:destination format:format];
    return YES;
}

// This describes audio-processing capability, not allowed HTML playbackRate.
// Zero and rates outside the effect's range require a silent transport window;
// the media clock must still advance under the actual requested finite rate.
static BOOL OrynRateAudible(double rate,BOOL preservesPitch) {
    double magnitude=fabs(rate);
    return isfinite(rate) && (preservesPitch
        ? magnitude>=1.0/32.0 && magnitude<=32.0
        : magnitude>=0.25 && magnitude<=4.0);
}

void *oryn_rate_sink_open(double sampleRate,unsigned channels,
    double playbackRate,int preservesPitch) {
    @autoreleasepool { @try {
        if (!isfinite(sampleRate) || sampleRate<8000 || sampleRate>192000
            || channels<1 || channels>8
            || !OrynRateAudible(playbackRate,preservesPitch!=0)) return NULL;
        OrynRateSink *sink=[OrynRateSink new];
        sink.lock=[NSLock new];
        sink.format=[[AVAudioFormat alloc] initWithCommonFormat:AVAudioPCMFormatFloat32
            sampleRate:sampleRate channels:channels interleaved:NO];
        sink.engine=[AVAudioEngine new];
        sink.player=[AVAudioPlayerNode new];
        sink.stretch=[AVAudioUnitTimePitch new];
        sink.varispeed=[AVAudioUnitVarispeed new];
        if (!sink.lock || !sink.format || !sink.engine || !sink.player
            || !sink.stretch || !sink.varispeed) return NULL;
        // Reverse transport supplies reversed PCM frames. Effects consume the
        // positive magnitude and keep channel ordering unchanged.
        sink.stretch.rate=preservesPitch?(float)fabs(playbackRate):1.0f;
        sink.stretch.pitch=0.0f;
        sink.varispeed.rate=preservesPitch?1.0f:(float)fabs(playbackRate);
        [sink.engine attachNode:sink.player];
        [sink.engine attachNode:sink.stretch];
        [sink.engine attachNode:sink.varispeed];
        NSError *error=nil;
        if (!OrynRateConnect(sink.engine,sink.player,sink.stretch,sink.format,&error)
            || !OrynRateConnect(sink.engine,sink.stretch,sink.varispeed,sink.format,&error)
            || !OrynRateConnect(sink.engine,sink.varispeed,sink.engine.mainMixerNode,sink.format,&error))
            return NULL;
        __weak OrynRateSink *weakSink=sink;
        // Capture real post-effect PCM. The callback owns one bounded latest
        // window, never author data or document/JS pointers. Graph integration
        // must consume sequence identities and report missed windows explicitly.
        [sink.varispeed installTapOnBus:0 bufferSize:1024 format:nil
            block:^(AVAudioPCMBuffer *pcm,AVAudioTime *when) {
                @autoreleasepool {
                    OrynRateSink *current=weakSink;
                    if (!current) return;
                    NSUInteger frames=pcm.frameLength;
                    NSUInteger count=pcm.format.channelCount;
                    double rate=pcm.format.sampleRate;
                    if (!frames || count<1 || count>8 || frames>32768
                        || frames*count*4>1024*1024 || !isfinite(rate)
                        || rate<8000 || rate>192000
                        || pcm.format.commonFormat!=AVAudioPCMFormatFloat32
                        || !pcm.floatChannelData) return;
                    [current.lock lock];
                    if (current.retired || current.sequence==UINT64_MAX) {
                        [current.lock unlock]; return;
                    }
                    [current.lock unlock];
                    NSMutableData *bytes=[NSMutableData dataWithLength:frames*count*4];
                    if (!bytes || !bytes.mutableBytes) return;
                    uint8_t *destination=bytes.mutableBytes;
                    BOOL interleaved=pcm.format.isInterleaved;
                    for (NSUInteger frame=0;frame<frames;frame++)
                        for (NSUInteger channel=0;channel<count;channel++) {
                            float sample=interleaved?pcm.floatChannelData[0][frame*count+channel]
                                :pcm.floatChannelData[channel][frame];
                            if (!isfinite(sample)) sample=0;
                            memcpy(destination+(frame*count+channel)*4,&sample,4);
                        }
                    [current.lock lock];
                    if (!current.retired && current.sequence<UINT64_MAX) {
                        if (current.latestPCM && current.dropped<UINT64_MAX) current.dropped++;
                        current.latestPCM=bytes;
                        current.outputRate=rate;
                        current.outputChannels=(unsigned)count;
                        current.sequence++;
                    }
                    [current.lock unlock];
                }
            }];
        if (![sink.engine startAndReturnError:&error]) return NULL;
        if (@available(macOS 27.0,*)) {
            if (![sink.player playAndReturnError:&error]) {
                [sink.engine stop];
                return NULL;
            }
        } else {
            @try { [sink.player play]; }
            @catch(NSException *exception) { [sink.engine stop]; return NULL; }
        }
        return (__bridge_retained void *)sink;
    } @catch(NSException *exception) { return NULL; } }
}

int oryn_rate_sink_write(void *opaque,const uint8_t *bytes,size_t size,
    float volume,int reverse) {
    @autoreleasepool {
        if (!opaque) return 0;
        OrynRateSink *sink=(__bridge OrynRateSink *)opaque;
        NSUInteger channels=sink.format.channelCount;
        if (!bytes || !size || size>1024*1024 || channels<1 || channels>8
            || size%(channels*4) || !isfinite(volume) || volume<0 || volume>1) return 0;
        NSUInteger frames=size/(channels*4);
        [sink.lock lock];
        if (sink.retired) { [sink.lock unlock]; return 0; }
        if (sink.queued>=8) { [sink.lock unlock]; return 2; }
        sink.queued++;
        [sink.lock unlock];
        AVAudioPCMBuffer *buffer=nil;
        @try {
            buffer=[[AVAudioPCMBuffer alloc] initWithPCMFormat:sink.format
                frameCapacity:(AVAudioFrameCount)frames];
            if (!buffer || !buffer.floatChannelData) @throw [NSException
                exceptionWithName:@"OrynAudioAllocation" reason:nil userInfo:nil];
            buffer.frameLength=(AVAudioFrameCount)frames;
            for (NSUInteger channel=0;channel<channels;channel++)
                for (NSUInteger frame=0;frame<frames;frame++) {
                    NSUInteger sourceFrame=reverse?frames-1-frame:frame;
                    float sample;
                    memcpy(&sample,bytes+(sourceFrame*channels+channel)*4,4);
                    buffer.floatChannelData[channel][frame]=isfinite(sample)?sample:0;
                }
            [sink.lock lock];
            if (sink.retired) {
                sink.queued--;
                [sink.lock unlock];
                return 0;
            }
            // Owner serializes close and writes; completion only updates the
            // inventory, and never publishes back to a retired document.
            [sink.lock unlock];
            sink.player.volume=volume;
            __weak OrynRateSink *weakSink=sink;
            [sink.player scheduleBuffer:buffer completionHandler:^{
                OrynRateSink *current=weakSink;
                if (!current) return;
                [current.lock lock];
                if (current.queued) current.queued--;
                [current.lock unlock];
            }];
            return 1;
        } @catch(NSException *exception) {
            [sink.lock lock];
            if (sink.queued) sink.queued--;
            [sink.lock unlock];
            return 0;
        }
    }
}
void oryn_rate_sink_close(void *opaque) {
    @autoreleasepool {
        if (!opaque) return;
        OrynRateSink *sink=(__bridge_transfer OrynRateSink *)opaque;
        [sink.lock lock];
        sink.retired=YES;
        sink.latestPCM=nil;
        [sink.lock unlock];
        [sink.varispeed removeTapOnBus:0];
        [sink.player stop];
        [sink.engine stop];
        // A new sink is needed for seek/rate/pitch discontinuities so neither
        // effect carries old buffered audio or processing history forward.
    }
}

// Returns owned JSON to the existing parent free contract. Only a parent
// owner-validated read API may expose this data; no direct worker pointer.
char *oryn_rate_sink_audio(void *opaque,uint64_t after) {
    @autoreleasepool {
        if (!opaque) return NULL;
        OrynRateSink *sink=(__bridge OrynRateSink *)opaque;
        [sink.lock lock];
        NSData *pcm=sink.latestPCM;
        uint64_t sequence=sink.sequence,dropped=sink.dropped;
        BOOL retired=sink.retired;
        double rate=sink.outputRate;
        unsigned channels=sink.outputChannels;
        [sink.lock unlock];
        if (after>sequence) return NULL;
        // Expensive serialization is outside the audio callback's lock. Failed
        // allocation retains original sample custody; a newer callback must
        // never be cleared by consumption of this older snapshot.
        BOOL pending=sequence<=after || !pcm;
        uint64_t missed=sequence>after?sequence-after-1:0;
        NSDictionary *row=@{ @"retired":@(retired),@"pending":@(pending),
            @"sequence":@(sequence),@"overwrittenWindows":@(dropped),
            @"missedSinceAfter":@(missed),@"sampleRate":@(rate),@"channels":@(channels),
            @"data":!pending?[pcm base64EncodedStringWithOptions:0]:@"" };
        NSData *json=[NSJSONSerialization dataWithJSONObject:row options:0 error:nil];
        if (!json || json.length>2*1024*1024) return NULL;
        char *copy=malloc(json.length+1);
        if (!copy) return NULL;
        memcpy(copy,json.bytes,json.length);
        copy[json.length]=0;
        // Non-consuming snapshot: independent graph consumers retain their own
        // last sequence. No reader can clear another reader's PCM observation.
        return copy;
    }
}

