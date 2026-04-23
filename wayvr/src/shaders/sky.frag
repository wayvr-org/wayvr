#version 450

layout(location = 0) in vec2 vUV;
layout(location = 0) out vec4 outColor;

float gauss(float x, float sigma)
{
    float t = x / sigma;
    return exp(-t * t);
}

float smoothBand(float x, float lo, float hi, float featherLo, float featherHi)
{
    float a = smoothstep(lo - featherLo, lo + featherLo, x);
    float b = 1.0 - smoothstep(hi - featherHi, hi + featherHi, x);
    return a * b;
}

float thinLine(float x, float halfWidth)
{
    float d  = abs(x);
    float aa = max(fwidth(x) * 1.5, 1e-5);

    float core = 1.0 - smoothstep(0.0, halfWidth + aa, d);
    float edge = 1.0 - smoothstep(halfWidth, halfWidth + aa, d);

    return core * edge;
}

float hash11(float p)
{
    return fract(sin(p * 127.1 + 311.7) * 43758.5453123);
}

float hash12(vec2 p)
{
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453123);
}

float noise1Periodic(float x, float period)
{
    float i0 = floor(x);
    float i1 = i0 + 1.0;
    float f  = fract(x);
    float u  = f * f * (3.0 - 2.0 * f);

    float a = hash11(mod(i0, period));
    float b = hash11(mod(i1, period));
    return mix(a, b, u);
}

float fbm1Periodic(float x, float period)
{
    float sum  = 0.0;
    float amp  = 0.5;
    float norm = 0.0;

    for (int i = 0; i < 5; ++i)
    {
        sum  += amp * noise1Periodic(x, period);
        norm += amp;
        x *= 2.0;
        period *= 2.0;
        amp *= 0.5;
    }

    return sum / norm;
}

float noise2TileX(vec2 p, float periodX)
{
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 u = f * f * (3.0 - 2.0 * f);

    float x0 = mod(i.x, periodX);
    float x1 = mod(i.x + 1.0, periodX);
    float y0 = i.y;
    float y1 = i.y + 1.0;

    float a = hash12(vec2(x0, y0));
    float b = hash12(vec2(x1, y0));
    float c = hash12(vec2(x0, y1));
    float d = hash12(vec2(x1, y1));

    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

float fbm2TileX(vec2 p, float periodX)
{
    float sum  = 0.0;
    float amp  = 0.5;
    float norm = 0.0;

    for (int i = 0; i < 4; ++i)
    {
        sum  += amp * noise2TileX(p, periodX);
        norm += amp;
        p *= 2.0;
        periodX *= 2.0;
        amp *= 0.5;
    }

    return sum / norm;
}

vec3 tonemapACESApprox(vec3 x)
{
    const float a = 2.51;
    const float b = 0.03;
    const float c = 2.43;
    const float d = 0.59;
    const float e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), 0.0, 1.0);
}

float independentPath(
    float u,
    float baseLat,
    float warpAmp0, float warpFreq0, float warpPhase0,
    float warpAmp1, float warpFreq1, float warpPhase1,
    float amp0,     float freq0,     float phase0,
    float amp1,     float freq1,     float phase1,
    float amp2,     float freq2,     float phase2,
    out float uw)
{
    uw = fract(u
        + warpAmp0 * (fbm1Periodic(u * warpFreq0 + warpPhase0, warpFreq0) - 0.5)
        + warpAmp1 * (fbm1Periodic(u * warpFreq1 + warpPhase1, warpFreq1) - 0.5));

    return baseLat
        + amp0 * (fbm1Periodic(uw * freq0 + phase0, freq0) - 0.5)
        + amp1 * (fbm1Periodic(uw * freq1 + phase1, freq1) - 0.5)
        + amp2 * (fbm1Periodic(uw * freq2 + phase2, freq2) - 0.5);
}

float independentPositive(
    float u,
    float baseValue,
    float amp0, float freq0, float phase0,
    float amp1, float freq1, float phase1)
{
    return baseValue
        + amp0 * fbm1Periodic(u * freq0 + phase0, freq0)
        + amp1 * fbm1Periodic(u * freq1 + phase1, freq1);
}

void main()
{
    float u = fract(vUV.x);
    float v = 0.5 - vUV.y;

    // background gradient
    float skyT = smoothstep(-0.50, 0.50, v);
    vec3 col = mix(vec3(0.00008, 0.00012, 0.00045),
                   vec3(0.0220, 0.0470, 0.0980),
                   skyT);

    col += gauss(v - 0.24, 0.30) * vec3(0.0045, 0.0100, 0.0220) * 0.28;

    // seamless flow field around 360 degrees
    float flowA = fbm1Periodic(u *  7.0 + 1.3,  7.0);
    float flowB = fbm1Periodic(u * 13.0 + 5.1, 13.0);

    float uu = fract(u
                   + 0.045 * (flowA - 0.5)
                   + 0.020 * (flowB - 0.5));

    float sA = fbm1Periodic(uu *  5.0 +  2.7,  5.0);
    float sB = fbm1Periodic(uu * 11.0 + 11.4, 11.0);
    float sC = fbm1Periodic(uu * 23.0 +  4.2, 23.0);
    float sD = fbm1Periodic(uu *  9.0 + 17.2,  9.0);
    float sE = fbm1Periodic(uu * 19.0 +  8.3, 19.0);

    float center    = -0.012
                    + 0.100 * (sA - 0.5)
                    + 0.045 * (sB - 0.5)
                    + 0.018 * (sC - 0.5);

    float thickness = 0.072 + 0.040 * sD;
    float skew      = 0.014 * (sE - 0.5);

    float mainLo = center - 0.035 - skew;
    float mainHi = center + thickness + 0.15 * skew;

    float veilU;
    float veilCenter = independentPath(
        u, 0.050,
         0.028,  7.0,  1.1,
        -0.015, 13.0,  5.4,
         0.050,  3.0,  2.3,
         0.030, 11.0,  8.1,
         0.010, 29.0,  5.7,
        veilU);

    float veilThickness = independentPositive(
        veilU, 0.011,
        0.010,  9.0, 14.2,
        0.006, 21.0,  3.6);

    float veilLo = veilCenter - 0.004;
    float veilHi = veilCenter + veilThickness;

    float auroraU;
    float auroraCenter = independentPath(
        u, 0.182,
         0.033,  5.0,  3.9,
         0.020, 17.0,  7.7,
         0.060,  4.0,  1.6,
         0.040,  9.0, 12.4,
         0.014, 27.0,  6.9,
        auroraU);

    float auroraThickness = independentPositive(
        auroraU, 0.034,
        0.014,  7.0,  4.1,
        0.008, 19.0, 15.2);

    float auroraLo = auroraCenter - 0.004;
    float auroraHi = auroraCenter + auroraThickness;

    float mistU;
    float mistCenter = independentPath(
        u, 0.255,
         0.020,  6.0,  2.8,
        -0.012, 15.0,  9.1,
         0.030,  4.0,  1.9,
         0.020, 10.0, 13.2,
         0.008, 26.0,  7.4,
        mistU);

    float electric0U;
    float electric0Center = independentPath(
        u, 0.004,
         0.052, 9.0,  2.4,
        -0.018, 1.0, 11.6,
         0.098, 3.0,  4.1,
         0.068, 1.0,  9.7,
         0.054, 2.0, 15.3,
        electric0U);

    float electric1U;
    float electric1Center = independentPath(
        u, 0.002,
         0.046, 7.0,  8.3,
         0.020, 7.0,  3.1,
         0.080, 4.0,  6.6,
         0.071, 2.0, 12.2,
         0.064, 1.0,  5.8,
        electric1U);

    float electric2U;
    float electric2Center = independentPath(
        u, 0.006,
         0.044, 3.0, 13.7,
        -0.017, 4.0,  6.4,
         0.061, 5.0, 10.9,
         0.082, 6.0,  4.2,
         0.075, 5.0, 14.8,
        electric2U);

    float mainTex    = fbm2TileX(vec2(uu * 20.0 +  7.0, (v - center)       *  8.0 + 13.0), 20.0);
    float fineTex    = fbm2TileX(vec2(uu * 36.0 + 17.0, (v - center)       * 14.0 + 29.0), 36.0);

    float veilTex    = fbm2TileX(vec2(veilU   * 14.0 +  5.0, (v - veilCenter)   *  9.0 + 41.0), 14.0);
    float auroraTex  = fbm2TileX(vec2(auroraU * 16.0 + 21.0, (v - auroraCenter) * 11.0 + 57.0), 16.0);
    float auroraFine = fbm2TileX(vec2(auroraU * 28.0 +  3.0, (v - auroraCenter) * 18.0 + 71.0), 28.0);

    float bodyMod    = 0.82 + 0.28 * mainTex   + 0.18 * fineTex;
    float veilMod    = 0.80 + 0.30 * veilTex;
    float auroraMod  = 0.78 + 0.24 * auroraTex + 0.16 * auroraFine;

    // nasks
    float aura         = gauss(v - center, 0.14);

    float mainBody     = smoothBand(v, mainLo, mainHi, 0.018, 0.024);
    float mainMid      = gauss(v - mix(mainLo, mainHi, 0.42), 0.55 * thickness);
    float mainHiEdge   = gauss(v - mainHi, 0.017);

    float veilBody     = smoothBand(v, veilLo, veilHi, 0.014, 0.020);
    float veilMid      = gauss(v - mix(veilLo, veilHi, 0.52), 0.60 * veilThickness);
    float veilLoEdge   = gauss(v - veilLo, 0.012);
    float veilHiEdge   = gauss(v - veilHi, 0.018);

    float auroraBody   = smoothBand(v, auroraLo, auroraHi, 0.016, 0.022);
    float auroraMid    = gauss(v - mix(auroraLo, auroraHi, 0.52), 0.62 * auroraThickness);
    float auroraLoEdge = gauss(v - auroraLo, 0.013);
    float auroraHiEdge = gauss(v - auroraHi, 0.020);

    float mist         = gauss(v - mistCenter, 0.085);

    float veilLat   = v - veilCenter;
    float auroraLat = v - auroraCenter;

    float veilColorNoise =
          0.60 * fbm2TileX(vec2(veilU   *  6.0 + 2.0, veilLat   *  72.0 + 11.0),  6.0)
        + 0.40 * fbm2TileX(vec2(veilU   * 14.0 + 7.0, veilLat   * 128.0 + 31.0), 14.0);

    float auroraColorU = fract(
        auroraU
        + 0.035 * (fbm1Periodic(auroraU *  9.0 + 2.7,  9.0) - 0.5)
        + 0.018 * (fbm1Periodic(auroraU * 21.0 + 6.1, 21.0) - 0.5)
    );

    vec3 auroraColorNoise = clamp(vec3(
        fbm1Periodic(auroraColorU *  5.0 +  1.3,  5.0),  // R
        fbm1Periodic(auroraColorU *  9.0 +  7.8,  9.0),  // G
        fbm1Periodic(auroraColorU * 17.0 + 12.4, 17.0)   // B
    ), 0.0, 1.0);

    veilColorNoise   = clamp(veilColorNoise,   0.0, 1.0);
    auroraColorNoise = clamp(auroraColorNoise, 0.0, 1.0);

    vec3 veilBaseColor = 0.2 * mix(vec3(0.070, 0.160, 0.460),
                               vec3(0.140, 0.620, 0.760),
                               0.55 * veilColorNoise);

    vec3 veilPeakColor = 0.5 * mix(vec3(0.110, 0.240, 0.620),
                               vec3(0.180, 0.900, 0.950),
                               0.65 * veilColorNoise);

    vec3 auroraBaseColor = 0.2 * vec3(0.100, 0.640, 0.720) * auroraColorNoise * auroraColorNoise;
    vec3 auroraPeakColor = 0.5 * vec3(0.180, 0.980, 1.050) * auroraColorNoise * auroraColorNoise;

    float electric0     = thinLine(v - electric0Center, 0.0001);
    float electric1     = thinLine(v - electric1Center, 0.00015);
    float electric2     = thinLine(v - electric2Center, 0.0002);

    float electric0Glow = gauss(v - electric0Center, 0.0040);
    float electric1Glow = gauss(v - electric1Center, 0.0048);
    float electric2Glow = gauss(v - electric2Center, 0.0052);

    float e0Energy = 0.82 + 0.28 * fbm1Periodic(electric0U * 41.0 +  6.2, 41.0);
    float e1Energy = 0.80 + 0.25 * fbm1Periodic(electric1U * 47.0 + 10.4, 47.0);
    float e2Energy = 0.80 + 0.25 * fbm1Periodic(electric2U * 39.0 + 14.9, 39.0);
                          
    // compose
    col += aura * (0.85 + 0.25 * mainTex) * vec3(0.014, 0.032, 0.080) * 0.95;

    col += mainBody   * bodyMod * vec3(0.070, 0.160, 0.420) * 0.5;
    col += mainMid    * bodyMod * vec3(0.120, 0.280, 0.740) * 0.4;
    col += mainHiEdge * bodyMod * vec3(0.105, 0.240, 0.620) * 0.1;

    col += veilBody   * veilMod * veilBaseColor * 1.05;
    col += veilMid    * veilMod * veilPeakColor * 0.90;
    col += veilLoEdge * veilMod * mix(veilBaseColor, veilPeakColor, 0.55) * 0.80;
    col += veilHiEdge * veilMod * vec3(0.100, 0.220, 0.580) * 0.20;

    col += auroraBody   * auroraMod * auroraBaseColor * 0.95;
    col += auroraMid    * auroraMod * auroraPeakColor * 0.78;
    col += auroraLoEdge * auroraMod * mix(auroraBaseColor, auroraPeakColor, 0.45) * 0.55;
    col += auroraHiEdge * auroraMod * vec3(0.080, 0.180, 0.480) * 0.18;

    col += mist * (0.52 + 0.28 * auroraTex) * vec3(0.035, 0.082, 0.220) * 0.20;

    col += electric0Glow * vec3(0.70, 1.10, 1.90) * 0.55;
    col += electric1Glow * mix(veilBaseColor,   veilPeakColor,   0.60) * 1.20;
    col += electric2Glow * mix(auroraBaseColor, auroraPeakColor, 0.65) * 1.35;

    col += electric0 * e0Energy * vec3(10.0, 10.3, 10.8);
    col += electric1 * e1Energy * vec3( 6.2,  6.8,  7.4);
    col += electric2 * e2Energy * vec3( 5.8,  6.5,  7.2);

    //TODO: skipped if rendering to a HDR swapchain
    col = tonemapACESApprox(col);

    outColor = vec4(col, 1.0);
}
