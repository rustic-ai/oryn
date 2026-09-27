// Included after OrynWebM. No URL, process-global override or author-accessible
// backend selector. Native test entry points use an explicit per-asset mode.
#define OrynVPXBudget (64u * 1024u * 1024u)
static size_t OrynVPXReservation(OrynWebM *media){
 // VP9's16 reference/work buffers, including border and alignment headroom.
 // Reserve the allowance before publication, even while VT is selected, so a
 // later native failure cannot silently grow beyond the registry reservation.
 uint64_t w=((uint64_t)media.width+351)&~31ull,h=((uint64_t)media.height+351)&~31ull;
 return (size_t)MIN((uint64_t)OrynVPXBudget,w*h*24);
}
typedef struct { size_t used; size_t limit; unsigned buffers; } OrynVPXBuffers;
static int OrynVPXGet(void *raw,size_t minimum,vpx_codec_frame_buffer_t *frame){
 OrynVPXBuffers *pool=raw;
 if(!pool||!frame||!minimum||minimum>pool->limit||pool->used>pool->limit-minimum||pool->buffers>=VP9_MAXIMUM_REF_BUFFERS+VPX_MAXIMUM_WORK_BUFFERS)return -1;
 uint8_t *data=calloc(1,minimum);if(!data)return -1;
 frame->data=data;frame->size=minimum;frame->priv=data;pool->used+=minimum;pool->buffers++;return 0;
}
static int OrynVPXRelease(void *raw,vpx_codec_frame_buffer_t *frame){
 OrynVPXBuffers *pool=raw;if(!pool||!frame)return -1;
 if(!frame->priv)return 0;
 if(frame->priv!=frame->data||frame->size>pool->used||!pool->buffers)return -1;
 free(frame->priv);pool->used-=frame->size;pool->buffers--;memset(frame,0,sizeof(*frame));return 0;
}
static void OrynVideoReset(OrynWebM *media){
 if(media->vp8Ready){vpx_codec_destroy(&media->vp8);media->vp8Ready=NO;}
 if(media->decoder){VTDecompressionSessionInvalidate(media->decoder);CFRelease(media->decoder);media->decoder=NULL;}
 if(media->image){CVPixelBufferRelease(media->image);media->image=NULL;}
 media->decodeError=0;
}
static BOOL OrynSoftwareSession(OrynWebM *media){
 OrynVideoReset(media);
 OrynVPXBuffers *pool=media->framePool;
 if(!pool){pool=calloc(1,sizeof(*pool));if(!pool)return NO;media->framePool=pool;}
 if(pool->used||pool->buffers)return NO;
 pool->limit=media->testMode==3?1:OrynVPXReservation(media);
 vpx_codec_dec_cfg_t config={1,media.width,media.height};
 vpx_codec_err_t status=vpx_codec_dec_init(&media->vp8,media->isVP8?vpx_codec_vp8_dx():vpx_codec_vp9_dx(),&config,0);
 media->softwareStatus=status;media->vp8Ready=status==VPX_CODEC_OK;
 if(!media->vp8Ready)return NO;
 if(!media->isVP8){status=vpx_codec_set_frame_buffer_functions(&media->vp8,OrynVPXGet,OrynVPXRelease,pool);media->softwareStatus=status;if(status!=VPX_CODEC_OK){OrynVideoReset(media);return NO;}}
 return YES;
}
static uint8_t OrynVideoByte(double value){return (uint8_t)lrint(fmin(255.,fmax(0.,value)));}
static BOOL OrynVPXColor(vpx_color_space_t space,vpx_color_range_t range,double *kr,double *kb){
 if(range!=VPX_CR_STUDIO_RANGE&&range!=VPX_CR_FULL_RANGE)return NO;
 switch(space){
  case VPX_CS_UNKNOWN:case VPX_CS_BT_601:case VPX_CS_SMPTE_170:*kr=.299;*kb=.114;return YES;
  case VPX_CS_BT_709:*kr=.2126;*kb=.0722;return YES;
  case VPX_CS_SMPTE_240:*kr=.212;*kb=.087;return YES;
  case VPX_CS_BT_2020:*kr=.2627;*kb=.0593;return YES;
  default:return NO; // RGB/reserved are not profile0 I420 output.
 }
}
static BOOL OrynVPXImage(OrynWebM *media,const vpx_image_t *decoded){
 if(decoded->fmt!=VPX_IMG_FMT_I420||decoded->bit_depth!=8||decoded->x_chroma_shift!=1||decoded->y_chroma_shift!=1||decoded->d_w!=media.width||decoded->d_h!=media.height||decoded->stride[0]<(int)media.width||decoded->stride[1]<(int)((media.width+1)/2)||decoded->stride[2]<(int)((media.width+1)/2)||!decoded->planes[0]||!decoded->planes[1]||!decoded->planes[2])return NO;
 double kr,kb;if(!OrynVPXColor(decoded->cs,decoded->range,&kr,&kb))return NO;
 CVPixelBufferRef image=NULL;if(CVPixelBufferCreate(kCFAllocatorDefault,media.width,media.height,kCVPixelFormatType_32BGRA,NULL,&image)!=kCVReturnSuccess)return NO;
 if(CVPixelBufferLockBaseAddress(image,0)!=kCVReturnSuccess){CVPixelBufferRelease(image);return NO;}
 uint8_t *base=CVPixelBufferGetBaseAddress(image);size_t stride=CVPixelBufferGetBytesPerRow(image);
 if(!base||stride<(size_t)media.width*4){CVPixelBufferUnlockBaseAddress(image,0);CVPixelBufferRelease(image);return NO;}
 double ys=decoded->range==VPX_CR_FULL_RANGE?1.:255./219.,cs=decoded->range==VPX_CR_FULL_RANGE?1.:255./224.,offset=decoded->range==VPX_CR_FULL_RANGE?0.:16.;
 for(uint32_t y=0;y<media.height;y++)for(uint32_t x=0;x<media.width;x++){
  double yy=(decoded->planes[0][(size_t)y*decoded->stride[0]+x]-offset)*ys;
  double cb=(decoded->planes[1][(size_t)(y/2)*decoded->stride[1]+x/2]-128)*cs,cr=(decoded->planes[2][(size_t)(y/2)*decoded->stride[2]+x/2]-128)*cs;
  double red=yy+2*(1-kr)*cr,blue=yy+2*(1-kb)*cb,green=(yy-kr*red-kb*blue)/(1-kr-kb);
  uint8_t *pixel=base+(size_t)y*stride+x*4;pixel[0]=OrynVideoByte(blue);pixel[1]=OrynVideoByte(green);pixel[2]=OrynVideoByte(red);pixel[3]=255;
 }
 CVPixelBufferUnlockBaseAddress(image,0);if(media->image)CVPixelBufferRelease(media->image);media->image=image;return YES;
}
typedef struct {const uint8_t *data;size_t bits,at;BOOL valid;} OrynVP9Bits;
static unsigned OrynVP9Read(OrynVP9Bits *r,unsigned n){if(n>24||r->at>r->bits||n>r->bits-r->at){r->valid=NO;return 0;}unsigned v=0;while(n--){v=(v<<1)|((r->data[r->at/8]>>(7-r->at%8))&1);r->at++;}return v;}
static BOOL OrynVP9FrameBounds(const uint8_t *bytes,size_t length,uint32_t width,uint32_t height){
 if(!length||length>UINT_MAX)return NO;OrynVP9Bits r={bytes,length*8,0,YES};
 if(OrynVP9Read(&r,2)!=2)return NO;unsigned low=OrynVP9Read(&r,1),high=OrynVP9Read(&r,1);if(low||high)return NO;
 if(OrynVP9Read(&r,1)){OrynVP9Read(&r,3);return r.valid;} // Existing references were admitted at this exact size.
 unsigned inter=OrynVP9Read(&r,1),show=OrynVP9Read(&r,1),resilient=OrynVP9Read(&r,1);BOOL explicitSize=YES;
 if(!inter){if(OrynVP9Read(&r,24)!=0x498342)return NO;unsigned color=OrynVP9Read(&r,3);if(color==VPX_CS_SRGB||color==VPX_CS_RESERVED)return NO;OrynVP9Read(&r,1);}
 else{
  unsigned intra=show?0:OrynVP9Read(&r,1);if(!resilient)OrynVP9Read(&r,2);
  if(intra){if(OrynVP9Read(&r,24)!=0x498342)return NO;OrynVP9Read(&r,8);}
  else{OrynVP9Read(&r,8);for(unsigned i=0;i<3;i++)OrynVP9Read(&r,4);for(unsigned i=0;i<3;i++)if(OrynVP9Read(&r,1)){explicitSize=NO;break;}}
 }
 if(explicitSize){uint32_t w=OrynVP9Read(&r,16)+1,h=OrynVP9Read(&r,16)+1;if(w!=width||h!=height)return NO;}
 // Render dimensions may differ from coded dimensions; they cannot allocate a
 // different decoder frame. Existing HTML object-fit uses admitted track size.
 if(OrynVP9Read(&r,1)){OrynVP9Read(&r,16);OrynVP9Read(&r,16);}return r.valid;
}
static BOOL OrynVP9PacketBounds(const uint8_t *bytes,size_t length,uint32_t width,uint32_t height){
 if(!length)return NO;uint8_t marker=bytes[length-1];
 if((marker&0xe0)!=0xc0)return OrynVP9FrameBounds(bytes,length,width,height);
 unsigned count=(marker&7)+1,magnitude=((marker>>3)&3)+1;size_t index=2+count*magnitude;
 if(index>length||bytes[length-index]!=marker)return NO;
 size_t at=0,p=length-index+1,body=length-index;
 for(unsigned frame=0;frame<count;frame++){size_t size=0;for(unsigned j=0;j<magnitude;j++)size|=(size_t)bytes[p++]<<(j*8);if(!size||at>body||size>body-at||!OrynVP9FrameBounds(bytes+at,size,width,height))return NO;at+=size;}
 return at==body;
}
static BOOL OrynSoftwarePacket(OrynWebM *media,const uint8_t *bytes,NSUInteger length){
 if(!media->vp8Ready||!length||length>UINT_MAX)return NO;
 // libvpx's build-time 8192x8192 limit bounds header-driven internal allocations;
 // VP9 frame callbacks additionally cap retained/reference pixels at64MiB.
 if(media->isVP8){
  if(length<3)return NO;
  if(!(bytes[0]&1)){if(length<10||bytes[3]!=0x9d||bytes[4]!=1||bytes[5]!=0x2a)return NO;uint32_t width=(bytes[6]|bytes[7]<<8)&0x3fff,height=(bytes[8]|bytes[9]<<8)&0x3fff;if(width!=media.width||height!=media.height)return NO;}
 }else{
  vpx_codec_stream_info_t info={0};info.sz=sizeof(info);
  vpx_codec_err_t probe=vpx_codec_peek_stream_info(vpx_codec_vp9_dx(),bytes,(unsigned int)length,&info);
  if(probe==VPX_CODEC_OK&&info.is_kf&&(info.w!=media.width||info.h!=media.height))return NO;
 }
 vpx_codec_err_t status=vpx_codec_decode(&media->vp8,bytes,(unsigned int)length,NULL,0);media->softwareStatus=status;if(status!=VPX_CODEC_OK)return NO;
 vpx_codec_iter_t iterator=NULL;vpx_image_t *decoded;
 while((decoded=vpx_codec_get_frame(&media->vp8,&iterator)))if(!OrynVPXImage(media,decoded)){media->softwareStatus=VPX_CODEC_UNSUP_BITSTREAM;return NO;}
 return YES; // Invisible frames update reference state without replacing displayed pixels.
}
static void OrynVP9TestBits(uint8_t *data,unsigned *at,unsigned value,unsigned count){while(count--){unsigned bit=(value>>count)&1;data[*at/8]|=bit<<(7-*at%8);(*at)++;}}
// Private native test seam. No production JS binding or environment toggle.
int oryn_webm_decoder_unit_checks(void){
 OrynVPXBuffers pool={0,32,0};vpx_codec_frame_buffer_t a={0},b={0};
 if(OrynVPXGet(&pool,24,&a)||OrynVPXGet(&pool,9,&b)==0||pool.used!=24||OrynVPXRelease(&pool,&a)||pool.used||pool.buffers)return 0;
 if(OrynVPXGet(&pool,32,&b)||OrynVPXRelease(&pool,&b)||OrynVPXRelease(&pool,&b))return 0;
 double kr,kb;if(!OrynVPXColor(VPX_CS_BT_709,VPX_CR_FULL_RANGE,&kr,&kb)||fabs(kr-.2126)>1e-9||fabs(kb-.0722)>1e-9)return 0;
 if(OrynVPXColor(VPX_CS_SRGB,VPX_CR_FULL_RANGE,&kr,&kb)||OrynVideoByte(-1)!=0||OrynVideoByte(256)!=255)return 0;
 // Exact profile0 keyframe bounds and superframe index validation without decoder allocation.
 uint8_t header[32]={0};unsigned at=0;
 OrynVP9TestBits(header,&at,2,2);OrynVP9TestBits(header,&at,0,3);
 OrynVP9TestBits(header,&at,0,1);OrynVP9TestBits(header,&at,1,1);OrynVP9TestBits(header,&at,0,1);
 OrynVP9TestBits(header,&at,0x498342,24);OrynVP9TestBits(header,&at,1,3);OrynVP9TestBits(header,&at,0,1);
 OrynVP9TestBits(header,&at,15,16);OrynVP9TestBits(header,&at,15,16);OrynVP9TestBits(header,&at,0,1);
 size_t size=(at+7)/8;
 if(!OrynVP9PacketBounds(header,size,16,16)||OrynVP9PacketBounds(header,size,17,16)||OrynVP9PacketBounds(header,1,16,16))return 0;
 header[0]|=0x20;if(OrynVP9PacketBounds(header,size,16,16))return 0;header[0]&=~0x20;
 uint8_t superframe[80]={0};memcpy(superframe,header,size);memcpy(superframe+size,header,size);
 superframe[2*size]=0xc1;superframe[2*size+1]=(uint8_t)size;superframe[2*size+2]=(uint8_t)size;superframe[2*size+3]=0xc1;
 if(!OrynVP9PacketBounds(superframe,2*size+4,16,16))return 0;
 superframe[2*size+2]++;if(OrynVP9PacketBounds(superframe,2*size+4,16,16))return 0;
 OrynWebM *probe=[OrynWebM new];probe.width=16;probe.height=16;
 if(!OrynSoftwareSession(probe)||!OrynSoftwareSession(probe))return 0;
 OrynVideoReset(probe);OrynVPXBuffers *owned=probe->framePool;if(owned->used||owned->buffers||probe->vp8Ready)return 0;
 return 1;
}
// Feed the real pinned compressed keyframe directly to the same software codec,
// bypassing the outer packet scanner. A second unindexed keyframe advertises a
// different width: only the per-actually-decoded-frame allocation guard can stop
// that second frame before resizing decoder context/reference storage.
int oryn_webm_unindexed_bounds_check(void *raw){@autoreleasepool{
 OrynWebM *source=(__bridge OrynWebM *)raw;
 if(!source||source->isVP8||!source.video.count||source.width>=8192)return 0;
 NSDictionary *packet=source.video.firstObject;
 size_t offset=[packet[@"offset"] unsignedIntegerValue],length=[packet[@"length"] unsignedIntegerValue];
 if(![packet[@"key"] boolValue]||offset>source.bytes.length||length>source.bytes.length-offset||length<10||length>UINT_MAX/2-8)return 0;
 const uint8_t *bytes=(const uint8_t *)source.bytes.bytes+offset;
 // Select the first compressed frame when the original packet has an index.
 uint8_t marker=bytes[length-1];
 if((marker&0xe0)==0xc0){unsigned count=(marker&7)+1,magnitude=((marker>>3)&3)+1;size_t index=2+count*magnitude;
  if(index>length||bytes[length-index]!=marker)return 0;
  size_t first=0;for(unsigned j=0;j<magnitude;j++)first|=(size_t)bytes[length-index+1+j]<<(8*j);
  if(first<10||first>length-index)return 0;length=first;
 }
 vpx_codec_stream_info_t info={0};info.sz=sizeof(info);
 if(vpx_codec_peek_stream_info(vpx_codec_vp9_dx(),bytes,(unsigned)length,&info)!=VPX_CODEC_OK||!info.is_kf||info.w!=source.width||info.h!=source.height)return 0;
 OrynWebM *probe=[OrynWebM new];probe.width=source.width;probe.height=source.height;
 if(!OrynSoftwareSession(probe))return 0;
 // Establish that the unmodified first frame is genuinely decodable.
 if(vpx_codec_decode(&probe->vp8,bytes,(unsigned)length,NULL,0)!=VPX_CODEC_OK)return 0;
 vpx_codec_iter_t iterator=NULL;if(!vpx_codec_get_frame(&probe->vp8,&iterator))return 0;
 if(!OrynSoftwareSession(probe))return 0;
 // Zero trailing padding prevents a payload's final byte from being mistaken
 // for a superframe index. The pinned decoder explicitly skips this padding.
 NSMutableData *joined=[NSMutableData dataWithLength:length*2+8];uint8_t *target=joined.mutableBytes;
 memcpy(target,bytes,length);memcpy(target+length,bytes,length);
 // Profile0 key header:8bits flags +24sync +3color +1range => width at bit36.
 for(unsigned j=0;j<16;j++){unsigned bit=36+j,mask=1u<<(7-bit%8);uint8_t *byte=target+length+bit/8;
  *byte=(uint8_t)((*byte&~mask)|(((source.width>>(15-j))&1u)?mask:0));
 }
 vpx_codec_err_t status=vpx_codec_decode(&probe->vp8,target,(unsigned)joined.length,NULL,0);
 const char *detail=vpx_codec_error_detail(&probe->vp8);
 BOOL rejected=status==VPX_CODEC_CORRUPT_FRAME&&detail&&strstr(detail,"Oryn admitted frame dimensions differ");
 OrynVideoReset(probe);OrynVPXBuffers *pool=probe->framePool;
 if(!rejected||pool->used||pool->buffers)return 0;
 // The failed decode must not poison a subsequently acquired native owner.
 if(!OrynSoftwareSession(probe)||vpx_codec_decode(&probe->vp8,bytes,(unsigned)length,NULL,0)!=VPX_CODEC_OK)return 0;
 OrynVideoReset(probe);return pool->used==0&&pool->buffers==0;
}}
