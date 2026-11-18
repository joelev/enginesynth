# Model versus published output

Written by `engsim validate --write`. Simulated full-throttle torque and power at each engine's published peak-torque and peak-power speeds (quality `live96`), against the published figures in its definition. Generic engines are not listed.

| Engine | Modelled on | Torque: simulated vs published | Power: simulated vs published |
|---|---|---|---|
| `ktm_620_lc4e` | KTM 620 LC4-E, 609 cc (620 Duke), c. 1998 | 55 vs 60 N·m at 5500 rpm (-9%) | 38 vs 40 kW at 7000 rpm (-6%) |
| `harley_twin_cam_88` | Harley-Davidson Twin Cam 88 (1450 cc), carburetted, 1999-2006 | 109 vs 117 N·m at 3500 rpm (-6%) | 50 vs 49 kW at 5200 rpm (+2%) |
| `ducati_1098_testastretta` | Ducati Testastretta Evoluzione 1099 cc L-twin, Ducati 1098 / 1098 S, 2007-2008 | 121 vs 123 N·m at 8000 rpm (-2%) | 113 vs 119 kW at 9750 rpm (-5%) |
| `yamaha_cp2_689` | Yamaha CP2 689 cc parallel twin, MT-07 / FZ-07, 2014 on | 68 vs 68 N·m at 6500 rpm (+0%) | 54 vs 55 kW at 9000 rpm (-1%) |
| `triumph_bonneville_865` | Triumph Bonneville T100 865 cc air-cooled parallel twin, carburetted, 2005-2007 | 68 vs 68 N·m at 6000 rpm (+1%) | 47 vs 47 kW at 7250 rpm (-0%) |
| `kawasaki_ex500` | Kawasaki EX500 (Ninja 500R / GPZ500S) 498 cc parallel twin, 1987-2009 | 45 vs 46 N·m at 8500 rpm (-3%) | 43 vs 44 kW at 9800 rpm (-3%) |
| `triumph_daytona_675` | Triumph Daytona 675 674.8 cc inline-three, 2006-2008 (up to VIN 381274) | 66 vs 72 N·m at 11750 rpm (-8%) | 84 vs 92 kW at 12500 rpm (-9%) |
| `toyota_4a_ge` | Toyota 4A-GE first generation (big port, T-VIS), European specification without catalyst (10.0:1), AE86 Corolla GT, 1983 on | 144 vs 142 N·m at 5200 rpm (+1%) | 93 vs 91 kW at 6600 rpm (+2%) |
| `honda_b18c5` | Honda B18C5, Acura Integra Type R (US), 1997-2001 | 180 vs 176 N·m at 7300 rpm (+2%) | 142 vs 145 kW at 8000 rpm (-3%) |
| `honda_f20c` | Honda F20C, Honda S2000 AP1, Japanese specification (11.7:1) as launched in 1999 | 212 vs 218 N·m at 7500 rpm (-3%) | 180 vs 184 kW at 8300 rpm (-2%) |
| `honda_k20a` | Honda K20A, Integra TYPE R (DC5), Japanese specification (11.5:1), 2001-2004 | 201 vs 206 N·m at 7000 rpm (-3%) | 160 vs 162 kW at 8000 rpm (-1%) |
| `honda_k20c1` | Honda K20C1 2.0 L DOHC VTEC turbocharged inline-four, Civic Type R (FK8), US specification, 2017-2021 | 402 vs 400 N·m at 3500 rpm (+0%) | 228 vs 228 kW at 6500 rpm (+0%) |
| `porsche_944s_m44_40` | Porsche M44/40 2.5 L 16-valve inline-four, 944 S, 1987-1988 | 233 vs 230 N·m at 4300 rpm (+1%) | 138 vs 140 kW at 6000 rpm (-2%) |
| `bmw_s14b23` | BMW S14B23 with catalytic converter, E30 M3, 1986-1989 (195 PS) | 226 vs 230 N·m at 4750 rpm (-2%) | 141 vs 143 kW at 6750 rpm (-1%) |
| `vw_07k_25_cbta` | Volkswagen 2.5 L inline-five, engine code CBTA/CBUA (07K), 2008-2014 | 245 vs 240 N·m at 4250 rpm (+2%) | 125 vs 125 kW at 5700 rpm (-0%) |
| `audi_rr_20v_turbo` | Audi RR 2.2 L DOHC 20-valve turbocharged inline-five, Audi quattro 20V (Ur-quattro), 1989-1991 | 307 vs 309 N·m at 1950 rpm (-1%) | 161 vs 162 kW at 5900 rpm (-0%) |
| `bmw_s54b32` | BMW S54B32, E46 M3, 2000-2006 | 365 vs 365 N·m at 4900 rpm (-0%) | 247 vs 252 kW at 7900 rpm (-2%) |
| `toyota_2jz_ge` | Toyota 2JZ-GE without VVT-i, 1991-1997 (Supra, Aristo/GS 300, SC 300) | 297 vs 279 N·m at 4800 rpm (+6%) | 155 vs 162 kW at 5800 rpm (-4%) |
| `buick_320_straight8` | Buick 320 cu in Fireball straight-eight, Series 70 Roadmaster, 1950 Dynaflow rating | 360 vs 380 N·m at 2000 rpm (-5%) | 116 vs 113 kW at 3600 rpm (+2%) |
| `vw_vr6_aaa` | Volkswagen VR6 2.8 L 12-valve, engine code AAA, 1991-1999 | 238 vs 235 N·m at 4200 rpm (+1%) | 121 vs 128 kW at 5800 rpm (-6%) |
| `nissan_vq35de` | Nissan VQ35DE, 350Z base engine, 2003-2005 | 365 vs 371 N·m at 4800 rpm (-2%) | 222 vs 214 kW at 6200 rpm (+4%) |
| `buick_3800_l36` | Buick 3800 Series II L36, 1995-2005 | n/a | n/a |
| `buick_3800_l67` | GM 3800 Series II L67 supercharged V6 with the Generation III Eaton M90, 1998-2003 rating (Regal GS, Grand Prix GTP, Park Avenue Ultra, Bonneville SSEi) | 366 vs 380 N·m at 3600 rpm (-4%) | 170 vs 179 kW at 5200 rpm (-5%) |
| `buick_231_oddfire` | Buick 231 cu in V6, odd-fire, 1975 to early 1977 | n/a | n/a |
| `subaru_ej253` | Subaru EJ253 2.5 L SOHC flat-four with i-AVLS, US 2006 Legacy / Outback 2.5i | 227 vs 229 N·m at 4400 rpm (-1%) | 134 vs 130 kW at 6000 rpm (+2%) |
| `vw_type1_1600dp` | Volkswagen Type 1 1600 dual-port (1584 cc, European 50 PS, engine code AD/AS), Beetle 1302 S / 1303 S, 1970-1980 | 100 vs 106 N·m at 2800 rpm (-5%) | 39 vs 37 kW at 4000 rpm (+4%) |
| `citroen_2cv_375` | Citroën 2CV flat-twin, type A, 375 cc (62 x 62 mm, 6.2:1, 9 ch at 3500 rpm), 2CV A and AU, 1949-1955 | 20 vs 20 N·m at 2000 rpm (+2%) | 7 vs 7 kW at 3500 rpm (+4%) |
| `citroen_2cv_425` | Citroën 2CV flat-twin, type A 53, 425 cc (66 x 62 mm, 7.5:1, 16 DIN-PS / 18 ch SAE at 5000 rpm), 2CV AZ / AZAM, February 1963 to February 1970 | 26 vs 25 N·m at 3500 rpm (+3%) | 12 vs 12 kW at 5000 rpm (+4%) |
| `citroen_2cv_435` | Citroën flat-twin type A 79/1 (plate AYA 2), 435 cc (68.5 x 59 mm, 8.5:1, 24 ch DIN / 26 ch SAE at 6750 rpm), 2CV 4 1970-1979 (Dyane from 1968) | 29 vs 28 N·m at 4000 rpm (+5%) | 18 vs 18 kW at 6750 rpm (+2%) |
| `citroen_2cv_602` | Citroën flat-twin type M 28/1, plate A 06/635, 602 cc (74 x 70 mm, 8.5:1, 29 ch DIN at 5750 rpm, two-stage Solex 26/35 CSIC), 2CV 6 September 1978 to July 1990 | 40 vs 39 N·m at 3500 rpm (+4%) | 22 vs 21 kW at 5750 rpm (+1%) |
| `porsche_m97_21` | Porsche M97.21 3.4 L flat-six, 987 Cayman S 2006-2008 and Boxster S 2007-2008 | 339 vs 340 N·m at 4400 rpm (-0%) | 217 vs 217 kW at 6250 rpm (-0%) |
| `porsche_m96_76` | Porsche M96/76 3.6 L flat-six, 996 GT3 Mk I, 1999-2001 | 386 vs 370 N·m at 5000 rpm (+4%) | 259 vs 265 kW at 7200 rpm (-2%) |
| `porsche_930_60` | Porsche 930/60 3.3 L turbocharged flat-six, 911 Turbo 3.3 (930), 1978 model year, rest of world | 409 vs 412 N·m at 4000 rpm (-1%) | 228 vs 221 kW at 5500 rpm (+3%) |
| `ford_voodoo_52` | Ford 5.2 L "Voodoo" V8, Shelby GT350 (2015-2020) | 578 vs 582 N·m at 4750 rpm (-1%) | 397 vs 392 kW at 7500 rpm (+1%) |
| `maserati_f136s_42` | Maserati F136 S 4.2 L V8, Quattroporte V DuoSelect, 2004-2008 | 453 vs 451 N·m at 4500 rpm (+1%) | 291 vs 294 kW at 7000 rpm (-1%) |
| `maserati_f136yg_47` | Maserati F136 YG 4.7 L V8, Quattroporte S, 2008-2010 | 504 vs 490 N·m at 4750 rpm (+3%) | 315 vs 317 kW at 7000 rpm (-1%) |
| `maserati_f154_38tt` | Maserati F154 AM 3.8 L twin-turbocharged V8, Quattroporte VI (M156) GTS, 2013-2016 (530 PS) | 658 vs 650 N·m at 2000 rpm (+1%) | 377 vs 390 kW at 6800 rpm (-3%) |
| `cosworth_dfv_1967` | Ford Cosworth DFV 3.0 L Formula One V8, 1967 specification (Lotus 49) | 322 vs 332 N·m at 8500 rpm (-3%) | 299 vs 302 kW at 9000 rpm (-1%) |
| `dodge_viper_84_v10` | Dodge / SRT Viper 8.4 L V10 (Gen V, 2013-2017) | 822 vs 813 N·m at 4950 rpm (+1%) | 470 vs 477 kW at 6150 rpm (-2%) |
| `lamborghini_52_v10` | Lamborghini 5.2 L V10, Gallardo LP560-4 (2008-2013) | 545 vs 540 N·m at 6500 rpm (+1%) | 405 vs 412 kW at 8000 rpm (-2%) |
| `bmw_s85b50` | BMW S85B50, E60 M5 / E63 M6, 2005-2010 | 531 vs 520 N·m at 6100 rpm (+2%) | 377 vs 373 kW at 7750 rpm (+1%) |
| `lexus_1lr_gue` | Lexus 1LR-GUE 4.8 L V10, LFA (2010-2012) | 495 vs 480 N·m at 7000 rpm (+3%) | 398 vs 412 kW at 8700 rpm (-3%) |
| `porsche_carrera_gt_v10` | Porsche Carrera GT 5.7 L V10, type 980/01 (2003-2006) | 593 vs 590 N·m at 5750 rpm (+0%) | 439 vs 450 kW at 8000 rpm (-3%) |
| `bmw_s70_2_v12` | BMW S70/2 6.1 L V12, McLaren F1 (1993-1998) | 654 vs 651 N·m at 5600 rpm (+0%) | 461 vs 461 kW at 7500 rpm (-0%) |
| `lamborghini_l539_v12` | Lamborghini L539 6.5 L V12, Aventador LP 700-4 (2011 specification) | 670 vs 690 N·m at 5500 rpm (-3%) | 525 vs 515 kW at 8250 rpm (+2%) |
| `vw_w12_6l` | Volkswagen Group 6.0 L W12, Audi A8 L W12 code BSB (2004 on) | 592 vs 580 N·m at 4000 rpm (+2%) | 332 vs 335 kW at 6200 rpm (-1%) |
| `cadillac_v16_452` | Cadillac Series 452 V-16, 7.4 L overhead-valve (1930-1937) | 428 vs 434 N·m at 1400 rpm (-1%) | 118 vs 123 kW at 3400 rpm (-4%) |
| `continental_w670` | Continental W670-6A / R-670-5 (Boeing-Stearman PT-17), 220 hp | n/a | 165 vs 164 kW at 2075 rpm (+1%) |
| `mazda_13b_fc_na` | Mazda 13B six-port, naturally aspirated (RX-7 FC Series 5, 1989-1991) | 188 vs 190 N·m at 4000 rpm (-1%) | 127 vs 119 kW at 7000 rpm (+6%) |
| `mazda_13b_msp_renesis` | Mazda 13B-MSP Renesis, high-power six-port (RX-8, 2003 launch specification) | 204 vs 216 N·m at 5500 rpm (-6%) | 183 vs 177 kW at 8500 rpm (+3%) |
| `gm_ls3` | GM LS3 6.2 L V8, Chevrolet Corvette C6 (2008-2013) | 572 vs 575 N·m at 4600 rpm (-1%) | 320 vs 321 kW at 5900 rpm (-0%) |
| `gm_ls9` | GM LS9 6.2 L supercharged V8, Chevrolet Corvette ZR1 (C6, 2009-2013) | 825 vs 819 N·m at 3800 rpm (+1%) | 460 vs 476 kW at 6500 rpm (-3%) |
| `dodge_hellcat_62` | Chrysler/SRT 6.2 L supercharged Hemi V8 "Hellcat", 2015 Dodge Challenger and Charger SRT Hellcat | 876 vs 881 N·m at 4800 rpm (-1%) | 510 vs 527 kW at 6000 rpm (-3%) |
| `chevrolet_lt1_350_1970` | Chevrolet 350 LT-1 (RPO LT1), 1970 Corvette, 370 hp SAE gross | 528 vs 515 N·m at 4000 rpm (+3%) | 275 vs 276 kW at 6000 rpm (-1%) |
| `ford_289_gt40` | Ford 289 small-block V8, GT40 Mk I competition specification (1965) | 426 vs 447 N·m at 5500 rpm (-5%) | 269 vs 283 kW at 6500 rpm (-5%) |
| `ford_289_gt40_180` | Ford 289 small-block V8, GT40 Mk I competition specification (1965) | 452 vs 447 N·m at 5500 rpm (+1%) | 278 vs 283 kW at 6500 rpm (-2%) |
| `yamaha_r1_crossplane` | Yamaha YZF-R1 cross-plane inline-four (model code 14B), 2009-2014; 2009 data | 102 vs 116 N·m at 10000 rpm (-12%) | 113 vs 134 kW at 12500 rpm (-16%) |
| `ducati_desmosedici_stradale` | Ducati Desmosedici Stradale 1103 cc V4, Panigale V4 / V4 S, 2018 | 120 vs 124 N·m at 10000 rpm (-3%) | 133 vs 158 kW at 13000 rpm (-16%) |
| `kawasaki_h2_750` | Kawasaki H2 Mach IV 750 (1972) | 67 vs 77 N·m at 6500 rpm (-14%) | 45 vs 55 kW at 6800 rpm (-18%) |
| `trabant_p601` | Trabant P 601, engine P 65/66 (594.5 cc, 26 DIN-PS, 7.5:1), 1974-1985 | 49 vs 54 N·m at 3000 rpm (-10%) | 20 vs 19 kW at 4200 rpm (+5%) |
