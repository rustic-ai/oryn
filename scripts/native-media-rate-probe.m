// macOS offline regression of the actual AVAudioEngine effect graph.
// Compile from the repository root with clang -fobjc-arc and frameworks
// Foundation, AVFoundation, AudioToolbox, CoreMedia. Put the executable in the
// current coordinator staging lane. Run with native media-service access.
// No audio buffers are submitted until hardware rendering is stopped and the
// engine is placed in offline mode. This is not signed-worker qualification.
#import <Foundation/Foundation.h>
#include "../crates/oryn-native/src/media_capture/rate_sink_macos.inc"
int main(void) {@autoreleasepool {
 for(NSNumber *pitch in @[@NO,@YES])for(NSNumber *rate in @[@1,@2,@(-2)]){
  void *raw=oryn_rate_sink_open(48000,2,rate.doubleValue,pitch.boolValue);
  if(!raw)return 2;
  OrynRateSink *sink=(__bridge OrynRateSink *)raw;
  [sink.player stop];[sink.engine stop];
  NSError *error=nil;
  if(![sink.engine enableManualRenderingMode:AVAudioEngineManualRenderingModeOffline
       format:sink.format maximumFrameCount:1024 error:&error]){
   NSLog(@"manual admission failed %@",error);oryn_rate_sink_close(raw);return 3;
  }
  if(![sink.engine startAndReturnError:&error]){oryn_rate_sink_close(raw);return 4;}
  [sink.player play];
  NSMutableData *input=[NSMutableData dataWithLength:24000*2*4];
  float *samples=input.mutableBytes;
  for(size_t frame=0;frame<24000;frame++)for(size_t channel=0;channel<2;channel++)
   samples[frame*2+channel]=0.25f*sin(2*M_PI*440*frame/48000.);
  if(oryn_rate_sink_write(raw,input.bytes,input.length,1,rate.doubleValue<0)!=1){oryn_rate_sink_close(raw);return 5;}
  AVAudioPCMBuffer *output=[[AVAudioPCMBuffer alloc] initWithPCMFormat:sink.format frameCapacity:1024];
  size_t first=SIZE_MAX,last=0,crossings=0,total=0;
  NSMutableData *rendered=[NSMutableData dataWithLength:48*1024*4];
  float *observed=rendered.mutableBytes;
  size_t active=0;
  for(size_t quantum=0;quantum<48;quantum++){
   AVAudioEngineManualRenderingStatus status=[sink.engine renderOffline:1024 toBuffer:output error:&error];
   if(status==AVAudioEngineManualRenderingStatusError){NSLog(@"render error %@",error);oryn_rate_sink_close(raw);return 6;}
   if(status!=AVAudioEngineManualRenderingStatusSuccess)continue;
   for(size_t frame=0;frame<output.frameLength;frame++,total++){
    float sample=output.floatChannelData[0][frame];
    if(fabs(sample)>0.005){if(first==SIZE_MAX)first=total;last=total;active++;}
    observed[total]=sample;
   }
  }
  double duration=first==SIZE_MAX?0:(last-first+1)/48000.;
  // Count true zero crossings in the central energetic region, excluding
  // startup/tail effects. An amplitude threshold on the crossing itself would
  // undercount perfectly valid phase-dependent crossings.
  size_t begin=first==SIZE_MAX?0:first+2048,end=last>2048?last-2048:0;
  for(size_t frame=begin+1;frame<end;frame++)
   if(observed[frame-1]<=0&&observed[frame]>0)crossings++;
  double frequency=end>begin?crossings*48000./(end-begin):0;
  NSLog(@"rate=%@ preservesPitch=%@ first=%zu last=%zu active=%zu crossings=%zu duration=%g frequency=%g",rate,pitch,first,last,active,crossings,duration,frequency);
  char *snapshot=oryn_rate_sink_audio(raw,0);
  if(!snapshot){oryn_rate_sink_close(raw);return 7;}free(snapshot);
  oryn_rate_sink_close(raw);
  double expectedDuration=0.5/fabs(rate.doubleValue);
  double expectedFrequency=pitch.boolValue?440:440*fabs(rate.doubleValue);
  if(fabs(duration-expectedDuration)>0.04
      ||fabs(frequency-expectedFrequency)>expectedFrequency*0.05)return 8;
 }
 return 0;
}}
