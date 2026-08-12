set -e
cd "$(dirname "$0")"
S=seq; W=960; H=540
cat $S/f*.gray > all.gray
mk(){ # name, vf
  ffmpeg -y -loglevel error -f rawvideo -pix_fmt gray -s ${W}x${H} -r 60 -i all.gray \
    -vf "$2" -c:v libx265 -crf 20 -preset veryfast -pix_fmt yuv420p -x265-params log-level=none "$1" 
  sz=$(stat -f%z "$1"); echo "$1  $(echo "scale=2; $sz*8/3/1000000" | bc) Mbps  ($((sz/1024)) KiB)"
}
# 1. grey card baseline (flat 50% grey, no texture)
ffmpeg -y -loglevel error -f lavfi -i "color=c=gray:s=3840x2160:r=60:d=3" -c:v libx265 -crf 20 -preset veryfast -pix_fmt yuv420p -x265-params log-level=none base.mp4
echo "base_flatgrey.mp4  $(echo "scale=2; $(stat -f%z base.mp4)*8/3/1000000" | bc) Mbps"
# 2. clouds upscaled 960->4K, blended over grey at 20% amplitude (typical subtle overlay)
mk clouds_up4k_a20.mp4 "scale=3840:2160:flags=lanczos,format=gray,geq=lum='128+0.20*(p(X,Y)-128)'"
# 3. clouds upscaled, full amplitude
mk clouds_up4k_a100.mp4 "scale=3840:2160:flags=lanczos"
# 4. clouds upscaled + native-res film grain (grain must loop too - here just to measure bitrate effect)
mk clouds_up4k_a20_grain.mp4 "scale=3840:2160:flags=lanczos,format=gray,geq=lum='128+0.20*(p(X,Y)-128)',noise=alls=12:allf=t"
