T=q=>2**(q/12), // This strategy sounds best with equal-tempered chords rather than just intonation
S=(p,t)=>t*p&255, // Bounded saw wave generator
C=t=>S(1,t)+S(T(7),t)+S(T(10),t)+S(2*T(3),t)>>2, // Make a chord
// C(t) // Original chord in saw waves
C(t-8)>C(t)?64:0 // "Autochord", play around with 8 in t-8 for different "thickness"