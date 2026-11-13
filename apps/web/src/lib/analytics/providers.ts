import { GOOGLE_ADS_ID } from '@app/lib/analytics/googleConversions';

// No-op dataLayer/gtag queue. Installing it without loading gtag.js keeps
// queued events from throwing or hitting the network when no Google IDs are
// configured.
const installGtagStub = () => {
  const script = document.createElement('script');
  script.innerHTML = `
    window.dataLayer = window.dataLayer || [];
    function gtag(){dataLayer.push(arguments);}
  `;
  document.head.appendChild(script);
};

// No-op fbq queue for the same reason, when no Meta Pixel ID is configured.
const installFbqStub = () => {
  const script = document.createElement('script');
  script.innerHTML =
    'window.fbq = window.fbq || function(){ (window._fbq = window._fbq || []).push(arguments); };';
  document.head.appendChild(script);
};

export const initializeGoogleAnalytics = () => {
  const G_ID = import.meta.env.VITE_GA_ID ?? '';
  const GTM_ID = import.meta.env.VITE_GTM_ID ?? '';

  if (!G_ID) installGtagStub();

  // Google Analytics
  if (G_ID) {
    const gaScript = document.createElement('script');
    gaScript.src = `https://www.googletagmanager.com/gtag/js?id=${G_ID}`;
    gaScript.async = true;
    document.head.appendChild(gaScript);

    // Registering the AW account on page load is what lets gtag pick up
    // ?gclid=… from the URL into the _gcl_aw cookie, so subsequent
    // gtag('event', 'conversion', ...) fires can be attributed to the ad click.
    const gaInit = document.createElement('script');
    gaInit.innerHTML = `
      window.dataLayer = window.dataLayer || [];
      function gtag(){dataLayer.push(arguments);}
      gtag('js', new Date());
      gtag('config', '${G_ID}', { send_page_view: false });
      ${GOOGLE_ADS_ID ? `gtag('config', '${GOOGLE_ADS_ID}');` : ''}
    `;
    document.head.appendChild(gaInit);
  }

  // Google Tag Manager
  if (GTM_ID) {
    const gtmScript = document.createElement('script');
    gtmScript.innerHTML = `
      (function(w,d,s,l,i){w[l]=w[l]||[];w[l].push({'gtm.start':
      new Date().getTime(),event:'gtm.js'});var f=d.getElementsByTagName(s)[0],
      j=d.createElement(s),dl=l!='dataLayer'?'&l='+l:'';j.async=true;j.src=
      'https://www.googletagmanager.com/gtm.js?id='+i+dl;f.parentNode.insertBefore(j,f);
      })(window,document,'script','dataLayer','${GTM_ID}');
    `;
    document.head.appendChild(gtmScript);
  }
};

export const initializeMetaPixel = () => {
  const PIXEL_ID = import.meta.env.VITE_META_PIXEL_ID ?? '';

  if (!PIXEL_ID) {
    installFbqStub();
    return;
  }

  const fbqInit = document.createElement('script');
  fbqInit.innerHTML = `
     !function(f,b,e,v,n,t,s)
     {if(f.fbq)return;n=f.fbq=function(){n.callMethod?
     n.callMethod.apply(n,arguments):n.queue.push(arguments)};
     if(!f._fbq)f._fbq=n;n.push=n;n.loaded=!0;n.version='2.0';
     n.queue=[];t=b.createElement(e);t.async=!0;
     t.src=v;s=b.getElementsByTagName(e)[0];
     s.parentNode.insertBefore(t,s)}(window, document,'script',
     'https://connect.facebook.net/en_US/fbevents.js');
     fbq.disablePushState = true;
     fbq('init', '${PIXEL_ID}');
   `;

  document.head.appendChild(fbqInit);

  const pixelImage = document.createElement('img');

  pixelImage.width = 1;
  pixelImage.height = 1;
  pixelImage.src = `https://www.facebook.com/tr?id=${PIXEL_ID}&ev=ViewContent&cd[content_name]=App%20NoScript&ev=PageView&noscript=1`;
  pixelImage.style.display = 'none';

  const pixelImageInit = document.createElement('noscript');
  pixelImageInit.append(pixelImage);

  document.head.appendChild(pixelImageInit);
};
