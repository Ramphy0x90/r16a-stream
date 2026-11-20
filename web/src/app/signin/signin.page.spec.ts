import { provideHttpClient } from '@angular/common/http';
import { provideHttpClientTesting } from '@angular/common/http/testing';
import { provideZonelessChangeDetection } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { provideTransloco } from '@jsverse/transloco';

import { SessionService } from '../core/auth/session.service';
import { SigninPage } from './signin.page';

describe('SigninPage', () => {
  function setup(inputs: { error?: string; returnTo?: string; ref?: string }) {
    TestBed.configureTestingModule({
      imports: [SigninPage],
      providers: [
        provideZonelessChangeDetection(),
        provideRouter([]),
        provideHttpClient(),
        provideHttpClientTesting(),
        provideTransloco({ config: { availableLangs: ['en'], defaultLang: 'en' } }),
      ],
    });
    const fixture = TestBed.createComponent(SigninPage);
    for (const [k, v] of Object.entries(inputs)) {
      fixture.componentRef.setInput(k, v);
    }
    fixture.detectChanges();
    return fixture;
  }

  it('shows only known error messages', () => {
    expect(setup({ error: 'denied' }).componentInstance.message()).toBe('denied');
    TestBed.resetTestingModule();
    expect(setup({ error: '<script>' }).componentInstance.message()).toBeNull();
  });

  it('shows a setup reference only in the request id format', () => {
    const ok = setup({ error: 'setup', ref: '0123456789abcdef0123456789abcdef' });
    expect(ok.componentInstance.reference()).toBe('0123456789abcdef0123456789abcdef');
    expect((ok.nativeElement as HTMLElement).querySelector('code')?.textContent).toBe(
      '0123456789abcdef0123456789abcdef',
    );
    TestBed.resetTestingModule();
    expect(
      setup({ error: 'setup', ref: 'offline_access is missing' }).componentInstance.reference(),
    ).toBeNull();
    TestBed.resetTestingModule();
    expect(
      setup({
        error: 'failed',
        ref: '0123456789abcdef0123456789abcdef',
      }).componentInstance.reference(),
    ).toBeNull();
  });

  it('starts the login with the remembered page', () => {
    const fixture = setup({ returnTo: '/title/3' });
    const signIn = vi
      .spyOn(TestBed.inject(SessionService), 'signIn')
      .mockImplementation(() => undefined);
    (fixture.nativeElement as HTMLElement).querySelector('button')?.click();
    expect(signIn).toHaveBeenCalledWith('/title/3');
  });
});
